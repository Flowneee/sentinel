use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs::File;

use serde::Deserialize;

use failure::Fail;

use futures::{
    future::{ok, Future},
    stream::{iter_ok, Stream},
};

use crate::{
    notifier::{self, Notifier},
    sentinel, BoxedStream,
};

#[derive(Debug, Fail)]
pub(crate) enum SentinelAppError {
    #[fail(display = "Unknown notifier type '{}'", ty)]
    UnknownNotifierType { ty: String },
    #[fail(display = "Unknown notifier name '{}'", name)]
    UnknownNotifierName { name: String },

    #[fail(display = "Unknown sentinel type '{}'", ty)]
    UnknownSentinelType { ty: String },
}

pub(crate) fn load_env_config() -> GlobalConfig {
    let path = match env::var("CONFIG") {
        Ok(x) => x,
        Err(env::VarError::NotPresent) => "./config.yml".into(),
        Err(e) => panic!("{}", e),
    };
    let file = File::open(path).unwrap();
    let yaml_value = serde_yaml::from_reader::<_, serde_yaml::Value>(file).unwrap();
    let merged_value = yaml_merge_keys::merge_keys_serde(yaml_value).unwrap();
    serde_yaml::from_value(merged_value).unwrap()
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct GlobalConfig {
    resources: Vec<sentinel::YamlConfig>,
    notifiers: Vec<notifier::YamlConfig>,
}

pub(crate) struct SentinelApp {
    notifiers: BTreeMap<String, Box<dyn Notifier>>,
    resources_streams: Vec<BoxedStream<(), Box<dyn Error + Send>>>,
}

impl SentinelApp {
    pub(crate) fn new(config: GlobalConfig) -> Result<Self, Box<dyn Fail>> {
        let notifiers = Self::notifiers_from_configs(config.notifiers)?;
        let sentinel_configs = config
            .resources
            .into_iter()
            .map(|x| {
                let senders = x
                    .notifiers
                    .iter()
                    .map(|notifier_name| {
                        notifiers
                            .get(notifier_name)
                            .map(|x| x.sender())
                            .ok_or_else(|| SentinelAppError::UnknownNotifierName {
                                name: notifier_name.clone(),
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(sentinel::Config {
                    interval: x.interval,
                    name: x.name,
                    type_: x.type_,
                    notifiers: senders,
                    config: x.config,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e: SentinelAppError| Box::new(e) as Box<dyn Fail>)?;
        let resources_streams = sentinel_configs
            .into_iter()
            .map(|x| match x.type_.as_ref() {
                "http" => sentinel::http::HttpSentinel::create_sentinel_stream(x),
                ty => Err(
                    Box::new(SentinelAppError::UnknownSentinelType { ty: ty.into() })
                        as Box<dyn Fail>,
                )?,
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            notifiers,
            resources_streams,
        })
    }

    fn notifiers_from_configs(
        configs: Vec<notifier::YamlConfig>,
    ) -> Result<BTreeMap<String, Box<dyn Notifier>>, Box<dyn Fail>> {
        configs
            .into_iter()
            .map(|config| {
                let name = config.name;
                let notifier = match config.type_.as_ref() {
                    // Add here new type of notifiers.
                    "smtp" => notifier::smtp::SmtpNotifier::from_config(config.config)?,
                    ty => Err(
                        Box::new(SentinelAppError::UnknownNotifierType { ty: ty.into() })
                            as Box<dyn Fail>,
                    )?,
                };
                Ok((name, notifier))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|v| v.into_iter().collect::<BTreeMap<_, _>>())
    }

    pub(crate) fn run(&mut self) {
        let num_of_resources = self.resources_streams.len();
        let streams = self.resources_streams.drain(..).collect::<Vec<_>>();
        let task = iter_ok(streams)
            .map(|stream| {
                stream
                    .for_each(|_| ok(()))
                    .map_err(|e| log::error!("{:?}", e))
            })
            .buffer_unordered(num_of_resources)
            .for_each(|_| Ok(()));
        tokio::run(task);
    }
}
