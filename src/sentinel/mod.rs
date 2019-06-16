use std::{error::Error, time::Duration};

use futures::{Async, Future, Poll, Stream};

use either::Either;
use tokio_timer::{sleep, Delay};

use serde::Deserialize;

use crate::{
    notifier::{Message, NotifierSender},
    BoxedFuture,
};

mod impls;
pub(crate) use impls::*;

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct YamlConfig {
    pub interval: u64,
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub notifiers: Vec<String>,
    pub config: serde_yaml::Value,
}

pub(crate) struct Config {
    pub interval: u64,
    pub name: String,
    pub type_: String,
    pub notifiers: Vec<Box<dyn NotifierSender>>,
    pub config: serde_yaml::Value,
}

trait ResourceError {
    fn description(&self) -> String;
}

enum ResourceErrorState<'a, E: ResourceError> {
    New(&'a E),
    Changed(&'a E, &'a E),
    Resolved(&'a E),
}

impl<'a, E: ResourceError> ResourceErrorState<'a, E> {
    fn create_message(&self, resource_name: &str) -> Message {
        match self {
            ResourceErrorState::New(e) => {
                let title = format!("Error (new) {}", resource_name);
                let body = format!(
                    "Resource {} report new error:\n{}",
                    resource_name,
                    e.description()
                );
                Message::new(title, body)
            }
            ResourceErrorState::Changed(e1, e2) => {
                let title = format!("Error (changed) {}", resource_name);
                let body = format!(
                    "Resource {} report changed error:\nOld error: {}\nNew error: {}",
                    resource_name,
                    e1.description(),
                    e2.description()
                );
                Message::new(title, body)
            }
            ResourceErrorState::Resolved(e) => {
                let title = format!("Error (resolved) {}", resource_name);
                let body = format!(
                    "Resource {} resolved error:\n{}",
                    resource_name,
                    e.description()
                );
                Message::new(title, body)
            }
        }
    }
}

trait SentinelImpl: Send {
    type ResourceOk;
    type ResourceErr;
    type SentinelErr;
    //type Config;

    fn produce_future(
        &self,
    ) -> BoxedFuture<Result<Self::ResourceOk, Self::ResourceErr>, Self::SentinelErr>;
    fn compare_errors(&self, left: &Self::ResourceErr, right: &Self::ResourceErr) -> bool;
    //fn from_config(config: Self::Config) -> Result<Self, Box<dyn Error>> where Self: Sized;
}

struct Sentinel<R, E: ResourceError, C> {
    inner: Either<BoxedFuture<Result<R, E>, C>, Delay>,
    sentinel_impl: Box<dyn SentinelImpl<ResourceOk = R, ResourceErr = E, SentinelErr = C>>,
    active_error: Option<E>,
    interval: Duration,
    notifiers: Vec<Box<dyn NotifierSender>>,
    resource_name: String,
}

impl<R, E: ResourceError, C: Error + Send + 'static> Stream for Sentinel<R, E, C> {
    type Item = ();
    type Error = Box<dyn Error + Send>;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        loop {
            match self.inner {
                Either::Left(ref mut fut) => {
                    return match fut.poll() {
                        Ok(Async::Ready(t)) => {
                            self.process_result(t);
                            self.inner = Either::Right(sleep(self.interval));
                            Ok(Async::Ready(Some(())))
                        }
                        Ok(Async::NotReady) => Ok(Async::NotReady),
                        Err(e) => return Err(Box::new(e) as Box<dyn Error + Send>),
                    }
                }
                Either::Right(ref mut fut) => {
                    match fut.poll() {
                        Ok(Async::Ready(_)) => (),
                        Ok(Async::NotReady) => return Ok(Async::NotReady),
                        Err(e) => return Err(Box::new(e) as Box<dyn Error + Send>),
                    };
                    self.inner = Either::Left(self.sentinel_impl.produce_future());
                }
            }
        }
    }
}

impl<R, E: ResourceError, C: Error + Send + 'static> Sentinel<R, E, C> {
    pub(crate) fn new(
        sentinel_impl: Box<dyn SentinelImpl<ResourceOk = R, ResourceErr = E, SentinelErr = C>>,
        interval: u64,
        notifiers: Vec<Box<dyn NotifierSender>>,
        resource_name: String,
    ) -> Self {
        Self {
            inner: Either::Left(sentinel_impl.produce_future()),
            sentinel_impl,
            active_error: None,
            interval: Duration::from_millis(interval),
            notifiers,
            resource_name,
        }
    }

    fn process_result(&mut self, res: Result<R, E>) {
        let msg = match (self.active_error.as_ref(), res) {
            // No active error and current observation is successful.
            (None, Ok(_)) => None,
            // No active error and current observation produced error.
            (None, Err(e)) => {
                let msg = Some(ResourceErrorState::New(&e).create_message(&self.resource_name));
                self.active_error = Some(e);
                msg
            }
            // Have active error and observation produced error.
            (Some(e1), Err(e2)) => {
                // If error changed, report that, Otherwise do nothing.
                if !self.sentinel_impl.compare_errors(e1, &e2) {
                    let msg = Some(
                        ResourceErrorState::Changed(e1, &e2).create_message(&self.resource_name),
                    );
                    self.active_error = Some(e2);
                    msg
                } else {
                    None
                }
            }
            // Have active error, and observation is successful.
            (Some(e), Ok(_)) => {
                let msg = Some(ResourceErrorState::Resolved(e).create_message(&self.resource_name));
                self.active_error = None;
                msg
            }
        };
        if let Some(msg) = msg {
            self.notifiers.iter().for_each(|notifier| {
                tokio::spawn(notifier.send_message(msg.clone()));
            });
        }
    }
}
