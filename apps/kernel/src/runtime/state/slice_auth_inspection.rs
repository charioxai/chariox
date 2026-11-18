//! One provider-auth inspection of the running Docker slices at a time. Each inspection runs
//! `docker exec` per slice on the blocking pool, so a flood of `ListSlices` requests that each ran
//! their own filled that pool (starving the Cloud presence post) and the host's memory with docker
//! processes. A caller arriving while one runs shares its result instead of starting another.
use std::sync::{Arc, Mutex};

use futures_util::future::{BoxFuture, FutureExt, Shared};

use crate::slice::SliceRecord;
use crate::slice_provider_auth::SliceProviderAuthSummary;

pub(crate) type DetectedProviderAuth = Vec<(SliceRecord, Vec<SliceProviderAuthSummary>)>;
type Outcome = Arc<Result<DetectedProviderAuth, String>>;
type Inspection = Shared<BoxFuture<'static, Outcome>>;

#[derive(Clone, Default)]
pub(super) struct SliceAuthInspections {
    current: Arc<Mutex<Option<Inspection>>>,
}

impl SliceAuthInspections {
    pub(super) async fn inspect(&self, candidates: Vec<SliceRecord>) -> Outcome {
        let inspection = {
            let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
            match current.as_ref().filter(|running| running.peek().is_none()) {
                Some(running) => running.clone(),
                None => {
                    let started = run(candidates).boxed().shared();
                    *current = Some(started.clone());
                    started
                }
            }
        };
        inspection.await
    }
}

async fn run(candidates: Vec<SliceRecord>) -> Outcome {
    Arc::new(
        tokio::task::spawn_blocking(move || {
            candidates
                .into_iter()
                .filter_map(|slice| {
                    crate::slice::inspect_local_docker_slice_provider_auth(&slice, "all", None)
                        .ok()
                        .map(|auth| (slice, auth))
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|error| format!("slice provider auth inspection task failed: {error}")),
    )
}
