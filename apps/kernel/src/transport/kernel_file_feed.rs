//! Native filesystem interests on the ordinary kernel resource subscription.
use crate::local::{KernelFileWatchParams, KernelResourceWatch};
use crate::machine_files::FileWatch;
use crate::runtime::command::{KernelCaller, KernelCommandSource};
use crate::runtime::router::CommandRouter;
use crate::transport::kernel_protocol::{KernelEvent, KernelResourceChange};
use std::sync::Arc;
use tokio::sync::mpsc;

pub(super) struct FileFeed {
    task: tokio::task::JoinHandle<()>,
    _permit: crate::runtime::state::FileWatchPermit,
}
impl Drop for FileFeed {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) async fn start(
    router: Arc<CommandRouter>,
    source: KernelCommandSource,
    caller: KernelCaller,
    watch: &KernelResourceWatch,
    events: mpsc::Sender<KernelEvent>,
) -> Result<(FileFeed, KernelEvent), String> {
    let params: KernelFileWatchParams = serde_json::from_value(watch.params.clone())
        .map_err(|e| format!("invalid file watch: {e}"))?;
    if params.paths.is_empty() || params.paths.len() > crate::machine_files::MAX_FILE_WATCH_PATHS {
        return Err("watch between 1 and 64 file paths per client".into());
    }
    let permit = router
        .runtime_state()
        .reserve_file_watches(&caller, params.paths.len())?;
    let guard = router
        .authorize_file_watch(source, caller)
        .map_err(|e| e.to_string())?;
    let runtime = router.runtime_state();
    enum Source {
        Local(FileWatch),
        Remote(Box<crate::runtime::state::RemoteFileWatch>),
    }
    let mut source = match params.kernel {
        None => Source::Local(FileWatch::new(&params.paths, guard)?),
        Some(kernel) => Source::Remote(Box::new(
            runtime
                .watch_remote_files(&kernel, params.paths.clone())
                .await
                .map_err(|e| e.to_string())?,
        )),
    };
    let params = watch.params.clone();
    let epoch = format!("{:032x}", rand::random::<u128>());
    let event = move |sequence: u64, error: Option<String>| KernelEvent::KernelResourceChanged {
        resource: "file_changes".into(),
        params: params.clone(),
        revision: format!("{epoch}:{sequence}"),
        body: KernelResourceChange::Snapshot {
            data: serde_json::json!({"resync":true,"error":error}),
        },
    };
    // Register before acknowledging and invalidating, so writes racing the first read
    // are queued behind it. A new epoch resyncs changes made while disconnected.
    let initial = event(0, None);
    let task = tokio::spawn(async move {
        let mut sequence = 0u64;
        loop {
            let changed = match &mut source {
                Source::Local(native) => native.changed().await,
                Source::Remote(remote) => remote.changes.changed().await.map_err(|e| e.to_string()),
            };
            sequence = sequence.wrapping_add(1);
            if events
                .send(event(sequence, changed.as_ref().err().cloned()))
                .await
                .is_err()
                || changed.is_err()
            {
                return;
            }
        }
    });
    Ok((
        FileFeed {
            task,
            _permit: permit,
        },
        initial,
    ))
}
