//! The home kernel drives every copy: it pulls slices from the leased agent's workspace into its
//! own agent's workspace, or pushes slices the other way. A dropped link is waited out and the
//! copy resumes where it stopped; anything else ends the copy with nothing published.
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;

use super::workspace_slices::{
    changed, discard, read_slice, staged_len, write_slice, SourceSlice, WriteSlice,
};
use super::Place;
use crate::runtime::kernel_files::StagedWrite;

/// Slices requested ahead of the one being written, as the Environment import does.
const SLICES_IN_FLIGHT: usize = 3;
/// How long one slice may keep failing for want of a link before the copy gives up; a relay or
/// forwarder restart reconnects well inside it.
#[cfg(not(test))]
const RESUME_WINDOW: Duration = Duration::from_secs(180);
#[cfg(test)]
const RESUME_WINDOW: Duration = Duration::from_secs(3);
/// How long the removal of an abandoned upload keeps waiting for the link.
#[cfg(not(test))]
const DISCARD_WINDOW: Duration = Duration::from_secs(30);
#[cfg(test)]
const DISCARD_WINDOW: Duration = Duration::from_secs(3);
const RETRY_DELAYS: [Duration; 4] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
];

/// Why the leased agent's kernel gave nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinkError {
    /// The link failed; the same request may succeed once it is back.
    Offline(String),
    /// The worker answered that it will not; repeating cannot change that.
    Refused(String),
}

/// The files of another kernel, read one slice at a time over a link.
pub(crate) trait RemoteSource {
    async fn read(&self, path: &str, offset: u64) -> Result<SourceSlice, LinkError>;
    /// Who is meant in an error, as the start of a sentence.
    fn peer(&self) -> String {
        "The leased agent's kernel".to_owned()
    }
}

/// A leased agent's workspace, reached over the execution-lease link.
pub(crate) trait RemoteWorkspace: RemoteSource {
    async fn write(&self, path: &str, slice: &WriteSlice<'_>) -> Result<StagedWrite, LinkError>;
    async fn discard(&self, path: &str) -> Result<(), LinkError>;
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CopyOptions {
    pub overwrite: bool,
    /// The digest the caller was told; a source with another one is not copied.
    pub expect_sha256: Option<String>,
    pub control: Control,
}

/// What a job that outlives its caller adds to a pull: it reports how far it is, it can be
/// stopped, it waits longer for a link, and it continues bytes an earlier attempt staged.
#[derive(Clone, Default)]
pub(crate) struct Control {
    pub cancel: CancelToken,
    /// Told the bytes of this file that have landed.
    pub progress: Option<Arc<dyn Fn(u64) + Send + Sync>>,
    /// Replaces the default time a copy waits for a dropped link.
    pub link_window: Option<Duration>,
    /// Continue what a stopped copy of the same file staged.
    pub resume: bool,
}

impl std::fmt::Debug for Control {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Control")
            .field("link_window", &self.link_window)
            .field("resume", &self.resume)
            .finish_non_exhaustive()
    }
}

/// Raised once; every copy holding a clone stops at its next slice.
#[derive(Clone)]
pub(crate) struct CancelToken(tokio::sync::watch::Receiver<bool>);

impl Default for CancelToken {
    fn default() -> Self {
        // The sender is dropped at once: this token is never raised.
        CancelToken(tokio::sync::watch::channel(false).1)
    }
}

impl CancelToken {
    pub(crate) fn new() -> (tokio::sync::watch::Sender<bool>, Self) {
        let (raise, token) = tokio::sync::watch::channel(false);
        (raise, CancelToken(token))
    }

    pub(crate) fn is_raised(&self) -> bool {
        *self.0.borrow()
    }

    pub(crate) async fn raised(&self) {
        let mut token = self.0.clone();
        if token.wait_for(|raised| *raised).await.is_err() {
            // Dropped without being raised: never.
            std::future::pending::<()>().await;
        }
    }
}

pub(crate) const CANCELED: &str = "The copy was canceled; nothing was published";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Copied {
    pub bytes: u64,
    pub sha256: String,
    /// The version of the source that was copied, which a move deletes only while it still is.
    pub revision: String,
}

/// Copies `source` on the remote kernel to `destination` in `local`. A copy that continued bytes
/// an earlier attempt staged and then failed the digest starts over once from the first byte.
pub(crate) async fn pull<R: RemoteSource, P: Place>(
    remote: &R,
    source: &str,
    local: &P,
    destination: &str,
    options: &CopyOptions,
) -> Result<Copied, String> {
    match pull_once(remote, source, local, destination, options).await {
        (Err(_), true) => {
            let fresh = CopyOptions {
                control: Control {
                    resume: false,
                    ..options.control.clone()
                },
                ..options.clone()
            };
            pull_once(remote, source, local, destination, &fresh)
                .await
                .0
        }
        (outcome, _) => outcome,
    }
}

/// One attempt, and whether it continued staged bytes.
async fn pull_once<R: RemoteSource, P: Place>(
    remote: &R,
    source: &str,
    local: &P,
    destination: &str,
    options: &CopyOptions,
) -> (Result<Copied, String>, bool) {
    let control = &options.control;
    let peer = remote.peer();
    let first = match patiently(&peer, control, || remote.read(source, 0)).await {
        Ok(first) => first,
        Err(reason) => return (Err(reason), false),
    };
    let Some(sha256) = first.sha256.clone() else {
        return (Err(format!("{peer} sent no digest")), false);
    };
    if let Err(reason) = admit(source, first.size_bytes, &sha256, options, local) {
        return (Err(reason), false);
    }
    let size = first.size_bytes;
    let step = first.bytes.len() as u64;
    if step == 0 && size > 0 {
        return (Err(changed(source)), false);
    }
    // Bytes of an earlier attempt are continued from where they end.
    let staged = if control.resume && step > 0 && size > step {
        staged_len(local, destination)
    } else {
        0
    };
    let resumed = staged > 0 && staged < size;
    let start = if resumed { staged } else { 0 };
    let mut written = if resumed {
        Ok(StagedWrite {
            received: start,
            sha256: None,
        })
    } else {
        land(local, destination, &first, 0, &sha256, options).await
    };
    if !resumed {
        report(control, first.bytes.len() as u64);
    } else {
        report(control, start);
    }
    let from = if resumed { start } else { step };
    let mut slices = futures_util::stream::iter((from..size).step_by(step.max(1) as usize))
        .map(|offset| {
            let peer = &peer;
            async move {
                (
                    offset,
                    patiently(peer, control, || remote.read(source, offset)).await,
                )
            }
        })
        .buffered(SLICES_IN_FLIGHT);
    while matches!(written, Ok(StagedWrite { sha256: None, .. })) {
        let Some((offset, slice)) = slices.next().await else {
            break;
        };
        written = match slice {
            Ok(slice)
                if slice.size_bytes == size
                    && slice.revision == first.revision
                    && slice.bytes.len() as u64 == step.min(size - offset) =>
            {
                let landed = land(local, destination, &slice, offset, &sha256, options).await;
                report(control, offset + slice.bytes.len() as u64);
                landed
            }
            Ok(_) => Err(changed(source)),
            Err(reason) => Err(reason),
        };
    }
    drop(slices);
    let outcome = match written {
        Ok(StagedWrite {
            sha256: Some(landed),
            ..
        }) if landed == sha256 => Ok(Copied {
            bytes: size,
            sha256,
            revision: first.revision.clone(),
        }),
        Ok(_) => Err(changed(source)),
        Err(reason) => Err(reason),
    };
    // A copy that was stopped on purpose keeps what it staged for a later resume only when the
    // caller asked to resume; any other end removes it.
    if outcome.is_err() && !(control.resume && is_link_failure(&outcome)) {
        discard(local, destination);
    }
    (outcome, resumed)
}

fn report(control: &Control, landed: u64) {
    if let Some(progress) = &control.progress {
        progress(landed);
    }
}

/// Whether the copy ended because the link stayed down, which a later attempt can continue.
fn is_link_failure(outcome: &Result<Copied, String>) -> bool {
    matches!(outcome, Err(reason) if reason.contains("stayed unreachable"))
}

/// Copies `source` in `local` to `destination` in the leased agent's workspace.
pub(crate) async fn push<R: RemoteWorkspace, P: Place>(
    local: &P,
    source: &str,
    remote: &R,
    destination: &str,
    options: &CopyOptions,
) -> Result<Copied, String> {
    let peer = remote.peer();
    let control = &options.control;
    let first = read_local(local, source, 0).await?;
    let sha256 = first
        .sha256
        .clone()
        .expect("the first slice carries the digest");
    admit(source, first.size_bytes, &sha256, options, local)?;
    let size = first.size_bytes;
    let mut slice = first;
    let mut offset = 0;
    let mut stalled = 0;
    loop {
        let write = WriteSlice {
            offset,
            total_size: size,
            sha256: &sha256,
            overwrite: options.overwrite,
            bytes: &slice.bytes,
        };
        let staged = patiently(&peer, control, || remote.write(destination, &write)).await;
        let staged = match staged {
            Ok(staged) => staged,
            Err(reason) => return Err(abandon(remote, destination, reason).await),
        };
        if let Some(landed) = staged.sha256 {
            return if landed == sha256 {
                Ok(Copied {
                    bytes: size,
                    sha256,
                    revision: slice.revision.clone(),
                })
            } else {
                Err(changed(source))
            };
        }
        // The worker names where it stands; after a lost answer that is not where this side was.
        stalled = if staged.received > offset {
            0
        } else {
            stalled + 1
        };
        if staged.received > size || stalled > 2 {
            let reason = format!("{peer} stopped taking `{destination}`; nothing was written");
            return Err(abandon(remote, destination, reason).await);
        }
        offset = staged.received;
        slice = match read_local(local, source, offset).await {
            Ok(next) if next.size_bytes == size && next.revision == slice.revision => next,
            Ok(_) => return Err(abandon(remote, destination, changed(source)).await),
            Err(reason) => return Err(abandon(remote, destination, reason).await),
        };
    }
}

/// Removes the unfinished upload of a copy that ended. When the link does not come back within
/// the discard window the upload stays on the leased agent's kernel, and the error says so.
async fn abandon<R: RemoteWorkspace>(remote: &R, destination: &str, reason: String) -> String {
    let never = CancelToken::default();
    match wait_for_link(&remote.peer(), DISCARD_WINDOW, &never, || {
        remote.discard(destination)
    })
    .await
    {
        Ok(()) => reason,
        Err(_) => format!(
            "{}. The unfinished upload (a hidden `.chariox-copy-*` file beside `{destination}`) may remain on the leased agent's kernel; the next copy to that path replaces it",
            reason.trim_end_matches('.')
        ),
    }
}

fn admit(
    source: &str,
    size: u64,
    sha256: &str,
    options: &CopyOptions,
    local: &impl Place,
) -> Result<(), String> {
    if size > local.max_file_bytes() {
        return Err(super::workspace_slices::too_large(
            source,
            size,
            local.max_file_bytes(),
        ));
    }
    match &options.expect_sha256 {
        Some(expected) if !expected.trim().eq_ignore_ascii_case(sha256) => Err(format!(
            "`{source}` has SHA-256 {sha256}, not the expected {}; nothing was copied",
            expected.trim()
        )),
        _ => Ok(()),
    }
}

async fn read_local<P: Place>(place: &P, path: &str, offset: u64) -> Result<SourceSlice, String> {
    let (place, path) = (place.clone(), path.to_owned());
    tokio::task::spawn_blocking(move || read_slice(&place, &path, offset))
        .await
        .map_err(|_| "The copy stopped unexpectedly; nothing was written".to_owned())?
}

/// Writes one pulled slice into the caller's place under the same rules a worker applies;
/// the last one publishes the file.
async fn land<P: Place>(
    local: &P,
    destination: &str,
    slice: &SourceSlice,
    offset: u64,
    sha256: &str,
    options: &CopyOptions,
) -> Result<StagedWrite, String> {
    let (local, destination, bytes, sha256) = (
        local.clone(),
        destination.to_owned(),
        slice.bytes.clone(),
        sha256.to_owned(),
    );
    let (total_size, overwrite) = (slice.size_bytes, options.overwrite);
    tokio::task::spawn_blocking(move || {
        write_slice(
            &local,
            &destination,
            &WriteSlice {
                offset,
                total_size,
                sha256: &sha256,
                overwrite,
                bytes: &bytes,
            },
        )
    })
    .await
    .map_err(|_| "The copy stopped unexpectedly; nothing was written".to_owned())?
}

/// Repeats `request` while the link is down, for at most the resume window.
async fn patiently<T, F, Fut>(peer: &str, control: &Control, request: F) -> Result<T, String>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, LinkError>>,
{
    wait_for_link(
        peer,
        control.link_window.unwrap_or(RESUME_WINDOW),
        &control.cancel,
        request,
    )
    .await
}

async fn wait_for_link<T, F, Fut>(
    peer: &str,
    window: Duration,
    cancel: &CancelToken,
    request: F,
) -> Result<T, String>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, LinkError>>,
{
    let deadline = tokio::time::Instant::now() + window;
    let mut attempt = 0;
    loop {
        let answer = tokio::select! {
            answer = request() => answer,
            () = cancel.raised() => return Err(CANCELED.to_owned()),
        };
        match answer {
            Ok(value) => return Ok(value),
            Err(LinkError::Refused(reason)) => return Err(reason),
            Err(LinkError::Offline(why)) => {
                let delay = RETRY_DELAYS[attempt.min(RETRY_DELAYS.len() - 1)];
                if tokio::time::Instant::now() + delay > deadline {
                    return Err(format!(
                        "{peer} stayed unreachable for {} s ({why}); nothing was written. Copy again once it is back",
                        window.as_secs()
                    ));
                }
                tokio::select! {
                    () = tokio::time::sleep(delay) => {}
                    () = cancel.raised() => return Err(CANCELED.to_owned()),
                }
                attempt += 1;
            }
        }
    }
}
