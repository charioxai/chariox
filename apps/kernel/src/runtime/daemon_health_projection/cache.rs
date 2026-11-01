use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::sync::OnceCell;

use super::{build_daemon_health_projection, DaemonHealthProjectionInput};
use crate::runtime::projection::DaemonHealthProjection;

/// One collector per kernel. Requests only clone its bounded published result.
#[derive(Clone, Default)]
pub(crate) struct DaemonHealthCache {
    published: Arc<OnceCell<Arc<RwLock<DaemonHealthProjection>>>>,
}

impl DaemonHealthCache {
    pub(crate) async fn snapshot(
        &self,
        input: DaemonHealthProjectionInput,
    ) -> DaemonHealthProjection {
        let last_event_id = input.last_event_id;
        let published = self
            .published
            .get_or_init(|| async move {
                let projection = collect(input.clone()).await;
                let published = Arc::new(RwLock::new(projection));
                let weak = Arc::downgrade(&published);
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        if weak.strong_count() == 0 {
                            break;
                        }
                        let projection = collect(input.clone()).await;
                        let Some(published) = weak.upgrade() else {
                            break;
                        };
                        *published.write().unwrap_or_else(|error| error.into_inner()) = projection;
                    }
                });
                published
            })
            .await;
        let mut projection = published
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        projection.metadata.last_event_id = last_event_id;
        projection
    }
}

async fn collect(input: DaemonHealthProjectionInput) -> DaemonHealthProjection {
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || runtime.block_on(build_daemon_health_projection(input)))
        .await
        .expect("daemon health collector panicked")
}
