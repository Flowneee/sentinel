use std::{sync::mpsc, thread};

use lettre::{
    smtp::authentication::Credentials, smtp::error::SmtpResult, SendableEmail, SmtpClient,
    Transport,
};
use lettre_email::Mailbox;

use futures::{future::join_all, sync::oneshot, Future, Poll};

use log::{debug, error, info};

use serde::Deserialize;

use failure::Fail;

use crate::{
    notifier::{Message, Notifier, NotifierSender},
    BoxedFuture,
};

#[derive(Debug, Fail)]
enum SmtpError {
    #[fail(display = "YAML deserialize error: {}", err)]
    YamlDeserializeError { err: serde_yaml::Error },
    #[fail(display = "SMTP client error: {}", err)]
    SmtpClientError { err: lettre::smtp::error::Error },
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct EmailIdent {
    pub address: String,
    pub name: Option<String>,
}

impl EmailIdent {
    pub(crate) fn new(address: String, name: Option<String>) -> Self {
        Self { address, name }
    }
}

impl Into<Mailbox> for EmailIdent {
    fn into(self) -> Mailbox {
        Mailbox {
            address: self.address,
            name: self.name,
        }
    }
}

pub(crate) struct SmtpNotifier {
    sender_thread: thread::JoinHandle<()>,
    sender: SmtpSender,
}

impl SmtpNotifier {
    pub(crate) fn shutdown(self) -> Result<(), mpsc::SendError<SmtpSenderMessage>> {
        let res = self.sender.sender.send(SmtpSenderMessage::Shutdown);
        self.sender_thread.join().unwrap();
        res
    }
}

impl Notifier for SmtpNotifier {
    fn sender(&self) -> Box<dyn NotifierSender> {
        Box::new(self.sender.clone())
    }

    fn from_config(config: serde_yaml::Value) -> Result<Box<dyn Notifier>, Box<dyn Fail>>
    where
        Self: Sized,
    {
        let smtp_config = serde_yaml::from_value(config)
            .map_err(|e| Box::new(SmtpError::YamlDeserializeError { err: e }) as Box<dyn Fail>)?;
        let SmtpConfig {
            host,
            login,
            pwd,
            ident,
            recipients,
        } = smtp_config;
        let creds = Credentials::new(login.clone(), pwd);
        let client = SmtpClient::new_simple(&host)
            .map_err(|e| Box::new(SmtpError::SmtpClientError { err: e }) as Box<dyn Fail>)?
            .credentials(creds);
        let (sender_thread, sender) = run_smtp_sender(client);
        Ok(Box::new(Self {
            sender_thread,
            sender: SmtpSender {
                sender,
                recipients,
                from: EmailIdent::new(login, ident),
            },
        }))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SmtpSender {
    sender: mpsc::Sender<SmtpSenderMessage>,
    recipients: Vec<EmailIdent>,
    from: EmailIdent,
}

impl SmtpSender {
    fn send_email(
        &self,
        email: lettre_email::Email,
    ) -> Result<SmtpResultFuture, mpsc::SendError<SmtpSenderMessage>> {
        let (sender, reciever) = oneshot::channel();
        self.sender
            .send(SmtpSenderMessage::Email(email.into(), sender))
            .map(|_| SmtpResultFuture::new(reciever))
    }
}

impl NotifierSender for SmtpSender {
    fn send_message(&self, msg: Message) -> BoxedFuture<(), ()> {
        let emails = self
            .recipients
            .iter()
            .map(|x| {
                lettre_email::Email::builder()
                    .to(x.clone())
                    .from(self.from.clone())
                    .subject(msg.title.clone())
                    .text(msg.body.clone())
                    .build()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let sender = self.clone();
        Box::new(
            join_all(emails.into_iter().filter_map(move |x| {
                debug!("Send email to SmtpNotifier: {:#?}", x);
                match sender.send_email(x) {
                    Ok(fut) => Some(
                        fut.map(|smtp_result| match smtp_result {
                            Ok(r) => debug!("SmtpNotifier response: {:#?}", r),
                            Err(e) => error!("SmtpNotifier error: {:#?}", e),
                        })
                        .map_err(|_| error!("oneshot to SmtpNotifier cancelled!")),
                    ),
                    Err(e) => {
                        error!("Failed to send email into receiver!");
                        debug!("Details: {:#?}", e);
                        None
                    }
                }
            }))
            .map(|_| ()),
        )
    }
}

pub(crate) enum SmtpSenderMessage {
    Email(SendableEmail, oneshot::Sender<SmtpResult>),
    Shutdown,
}

fn run_smtp_sender(
    smtp_client: SmtpClient,
) -> (thread::JoinHandle<()>, mpsc::Sender<SmtpSenderMessage>) {
    let (sender, reciever) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut transport = smtp_client.transport();
        for msg in reciever.iter() {
            match msg {
                SmtpSenderMessage::Shutdown => {
                    info!("SMTP sender is shut down");
                    return;
                }
                SmtpSenderMessage::Email(email, sender) => {
                    if let Err(v) = sender.send(transport.send(email)) {
                        error!("Failed to send SmtpResult back: {:?}", v);
                    }
                }
            }
        }
    });
    (handle, sender)
}

pub(crate) struct SmtpResultFuture {
    inner: oneshot::Receiver<SmtpResult>,
}

impl SmtpResultFuture {
    fn new(receiver: oneshot::Receiver<SmtpResult>) -> Self {
        Self { inner: receiver }
    }
}

impl Future for SmtpResultFuture {
    type Item = SmtpResult;
    type Error = oneshot::Canceled;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.inner.poll()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SmtpConfig {
    pub host: String,
    pub login: String,
    pub pwd: String,
    pub ident: Option<String>,
    pub recipients: Vec<EmailIdent>,
}
