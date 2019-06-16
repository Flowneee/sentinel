use serde::Deserialize;

use failure::Fail;

use crate::BoxedFuture;

mod impls;

pub(crate) use impls::*;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct YamlConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub config: serde_yaml::Value,
}

#[derive(Clone)]
pub(crate) struct Message {
    title: String,
    body: String,
}

impl Message {
    pub(crate) fn new(title: String, body: String) -> Self {
        Self { title, body }
    }
}

pub(crate) trait Notifier {
    fn sender(&self) -> Box<dyn NotifierSender>;
    fn from_config(config: serde_yaml::Value) -> Result<Box<dyn Notifier>, Box<dyn Fail>>
    where
        Self: Sized;
}

pub(crate) trait NotifierSender: Send {
    fn send_message(&self, msg: Message) -> BoxedFuture<(), ()>;
}
