use std::{convert::TryFrom, error::Error};

use reqwest::{
    r#async::{Client, ClientBuilder, Response},
    Url,
};

use futures::Future;

use serde::Deserialize;

use failure::Fail;

use crate::{
    sentinel::{Config, ResourceError, Sentinel, SentinelImpl},
    BoxedFuture, BoxedStream,
};

#[derive(Debug, Fail)]
pub(crate) enum HttpSentinelError {
    // Resource failures
    #[fail(display = "Reqwest HTTP error: {}", err)]
    ReqwestHttpError { err: reqwest::Error },
    #[fail(display = "Non-successful HTTP code: {}", code)]
    NonSuccessfulHttpCode { code: u16 },

    // Build failures
    #[fail(display = "Invalid status code: {}", code)]
    InvalidStatusCode { code: u16 },
    #[fail(display = "YAML deserialize error: {}", err)]
    YamlDeserializeError { err: serde_yaml::Error },
    #[fail(display = "Reqwest client error: {}", err)]
    ReqwestClientError { err: reqwest::Error },
    #[fail(display = "Url parse error: {}", err)]
    UrlParseError { err: reqwest::UrlError },
}

#[derive(Deserialize, Clone, Debug)]
enum HttpCodesRaw {
    Success(Vec<u16>),
    Error(Vec<u16>),
}

#[derive(Deserialize, Clone, Debug)]
struct HttpSentinelConfig {
    url: String,
    codes: HttpCodesRaw,
}

#[derive(Clone)]
enum HttpCodes {
    Success(Vec<reqwest::StatusCode>),
    Error(Vec<reqwest::StatusCode>),
}

impl TryFrom<HttpCodesRaw> for HttpCodes {
    type Error = HttpSentinelError;

    fn try_from(value: HttpCodesRaw) -> Result<Self, HttpSentinelError> {
        let convert_vec_u16_to_code = |v: Vec<u16>| -> Result<_, _> {
            v.into_iter()
                .map(|x| reqwest::StatusCode::from_u16(x).map_err(|_| x))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|code| HttpSentinelError::InvalidStatusCode { code })
        };
        Ok(match value {
            HttpCodesRaw::Success(codes) => HttpCodes::Success(convert_vec_u16_to_code(codes)?),
            HttpCodesRaw::Error(codes) => HttpCodes::Error(convert_vec_u16_to_code(codes)?),
        })
    }
}

pub(crate) struct HttpSentinel {
    url: Url,
    client: Client,
    codes: HttpCodes,
}

impl HttpSentinel {
    pub(crate) fn create_sentinel_stream(
        config: Config,
    ) -> Result<BoxedStream<(), Box<dyn Error + Send>>, Box<dyn Fail>> {
        let http_config: HttpSentinelConfig =
            serde_yaml::from_value(config.config).map_err(|e| {
                Box::new(HttpSentinelError::YamlDeserializeError { err: e }) as Box<dyn Fail>
            })?;
        let client = ClientBuilder::new().build().map_err(|e| {
            Box::new(HttpSentinelError::ReqwestClientError { err: e }) as Box<dyn Fail>
        })?;
        let url = Url::parse(&http_config.url)
            .map_err(|e| Box::new(HttpSentinelError::UrlParseError { err: e }) as Box<dyn Fail>)?;;
        let codes =
            HttpCodes::try_from(http_config.codes).map_err(|e| Box::new(e) as Box<dyn Fail>)?;
        let sentinel_impl = Box::new(Self { url, client, codes });

        let sent = Sentinel::new(
            sentinel_impl,
            config.interval,
            config.notifiers,
            http_config.url,
        );
        Ok(Box::new(sent))
    }
}

impl SentinelImpl for HttpSentinel {
    type ResourceOk = Response;
    type ResourceErr = HttpSentinelError;
    type SentinelErr = reqwest::Error;

    fn produce_future(
        &self,
    ) -> BoxedFuture<Result<Self::ResourceOk, Self::ResourceErr>, Self::SentinelErr> {
        let codes = self.codes.clone();
        Box::new(
            self.client
                .get(self.url.clone())
                .send()
                .map_err(|e| HttpSentinelError::ReqwestHttpError { err: e })
                .and_then(move |res| match codes {
                    HttpCodes::Success(ref codes) => {
                        if !codes.contains(&res.status()) {
                            Err(HttpSentinelError::NonSuccessfulHttpCode {
                                code: res.status().as_u16(),
                            })
                        } else {
                            Ok(res)
                        }
                    }
                    HttpCodes::Error(ref codes) => {
                        if codes.contains(&res.status()) {
                            Err(HttpSentinelError::NonSuccessfulHttpCode {
                                code: res.status().as_u16(),
                            })
                        } else {
                            Ok(res)
                        }
                    }
                })
                .then(|res| dbg!(Ok(res))),
        )
    }

    fn compare_errors(&self, left: &Self::ResourceErr, right: &Self::ResourceErr) -> bool {
        match (left, right) {
            (
                HttpSentinelError::NonSuccessfulHttpCode { code: l },
                HttpSentinelError::NonSuccessfulHttpCode { code: r },
            ) => l == r,
            (
                HttpSentinelError::NonSuccessfulHttpCode { .. },
                HttpSentinelError::ReqwestHttpError { .. },
            ) => false,
            (
                HttpSentinelError::ReqwestHttpError { .. },
                HttpSentinelError::NonSuccessfulHttpCode { .. },
            ) => false,
            // TODO: make correct comparsion
            _ => true,
        }
    }
}

impl ResourceError for HttpSentinelError {
    fn description(&self) -> String {
        format!("{}", self)
    }
}
