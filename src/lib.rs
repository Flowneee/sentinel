use tokio::prelude::*;

mod app;
mod notifier;
mod sentinel;

/// Generic boxed future.
pub(crate) type BoxedFuture<R, E> = Box<dyn Future<Item = R, Error = E> + Send>;
pub(crate) type BoxedStream<R, E> = Box<dyn Stream<Item = R, Error = E> + Send>;

pub fn main() {
    env_logger::init();

    // let global_config = load_env_config();

    // use notifier::Notifier;
    // let smtp_notifier = notifier::smtp::SmtpNotifier::from_config(
    //     serde_yaml::from_value((global_config.notifiers.get("smtp").unwrap()).clone()).unwrap(),
    // );

    // let mut sentinel_config: sentinel::Config =
    //     serde_yaml::from_value((global_config.resources.get("localhost").unwrap()).clone())
    //         .unwrap();
    // sentinel_config.notifiers.push(smtp_notifier.sender());

    // let http_sentinel =
    //     sentinel::http::HttpSentinel::create_sentinel_stream(sentinel_config).unwrap();
    // let task = http_sentinel
    //     .for_each(|_| ok(()))
    //     .map_err(|e| println!("{:?}", e));
    // tokio::run(task);

    // smtp_notifier.shutdown().unwrap();
    let config = app::load_env_config();
    let mut app = app::SentinelApp::new(config).unwrap();
    app.run()
}
