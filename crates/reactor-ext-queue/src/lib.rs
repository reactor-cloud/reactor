mod http;
mod store;

use reactor_core::{ExtensionInfo, ExtensionSql, Hook, Kernel};
use std::sync::Arc;
use std::time::Duration;

pub use store::{
    archive, create_queue, list_queues, list_subscriptions, peek, read, remove, send, subscribe,
    unsubscribe, QueueError,
};

pub fn register<S>(kernel: &mut Kernel<S>)
where
    S: Clone + Send + Sync + 'static,
{
    kernel.merge(http::router::<S>());
    kernel.hook(Hook {
        name: "queue".into(),
        every: Duration::from_secs(1),
        run: Arc::new(|ctx| Box::pin(store::drain(ctx))),
    });
    kernel.sql.push(ExtensionSql {
        version: "x_queue/001_queue.sql".into(),
        body: include_str!("../project/001_queue.sql").into(),
    });
    kernel.info.push(ExtensionInfo {
        name: "queue".into(),
        routes: vec![
            "POST /queue/v1/queues".into(),
            "GET /queue/v1/queues".into(),
            "POST /queue/v1/queues/{name}/send".into(),
            "POST /queue/v1/queues/{name}/read".into(),
            "POST /queue/v1/queues/{name}/delete".into(),
            "POST /queue/v1/queues/{name}/archive".into(),
            "GET /queue/v1/queues/{name}/peek".into(),
            "POST /queue/v1/queues/{name}/subscriptions".into(),
            "DELETE /queue/v1/queues/{name}/subscriptions".into(),
        ],
    });
}
