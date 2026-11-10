//! The home kernel drives every copy: it pulls slices from the leased agent's workspace into its
//! own agent's workspace, or pushes slices the other way. A dropped link is waited out and the
//! copy resumes where it stopped; anything else ends the copy with nothing published.
use std::time::Duration;

use futures_util::StreamExt;

use super::workspace_slices::{changed, discard, read_slice, write_slice, SourceSlice, WriteSlice};
use super::{Workspace, MAX_FILE_BYTES};
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

/// A leased agent's workspace, reached over the execution-lease link.
pub(crate) trait RemoteWorkspace {
    async fn read(&self, path: &str, offset: u64) -> Result<SourceSlice, LinkError>;
    async fn write(&self, path: &str, slice: &WriteSlice<'_>) -> Result<StagedWrite, LinkError>;
    async fn discard(&self, path: &str) -> Result<(), LinkError>;
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CopyOptions {
    pub overwrite: bool,
    /// The digest the caller was told; a source with another one is not copied.
    pub expect_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Copied {
    pub bytes: u64,
    pub sha256: String,
}

/// Copies `source` in the leased agent's workspace to `destination` in `local`.
pub(crate) async fn pull<R: RemoteWorkspace>(
    remote: &R,
    source: &str,
    local: &Workspace,
    destination: &str,
    options: &CopyOptions,
) -> Result<Copied, String> {
    let first = patiently(|| remote.read(source, 0)).await?;
    let sha256 = first
        .sha256
        .clone()
        .ok_or_else(|| "The leased agent's kernel sent no digest".to_owned())?;
    admit(source, first.size_bytes, &sha256, options)?;
    let size = first.size_bytes;
    let step = first.bytes.len() as u64;
    if step == 0 && size > 0 {
        return Err(changed(source));
    }
    let mut written = land(local, destination, &first, 0, &sha256, options).await;
    let mut slices = futures_util::stream::iter((step..size).step_by(step.max(1) as usize))
        .map(|offset| async move { (offset, patiently(|| remote.read(source, offset)).await) })
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
                land(local, destination, &slice, offset, &sha256, options).await
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
        }),
        Ok(_) => Err(changed(source)),
        Err(reason) => Err(reason),
    };
    if outcome.is_err() {
        discard(local, destination);
    }
    outcome
}

/// Copies `source` in `local` to `destination` in the leased agent's workspace.
pub(crate) async fn push<R: RemoteWorkspace>(
    local: &Workspace,
    source: &str,
    remote: &R,
    destination: &str,
    options: &CopyOptions,
) -> Result<Copied, String> {
    let first = read_local(local, source, 0).await?;
    let sha256 = first
        .sha256
        .clone()
        .expect("the first slice carries the digest");
    admit(source, first.size_bytes, &sha256, options)?;
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
        let staged = patiently(|| remote.write(destination, &write)).await;
        let staged = match staged {
            Ok(staged) => staged,
            Err(reason) => return Err(abandon(remote, destination, reason).await),
        };
        if let Some(landed) = staged.sha256 {
            return if landed == sha256 {
                Ok(Copied {
                    bytes: size,
                    sha256,
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
            let reason = format!(
                "The leased agent's kernel stopped taking `{destination}`; nothing was written"
            );
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
    match wait_for_link(DISCARD_WINDOW, || remote.discard(destination)).await {
        Ok(()) => reason,
        Err(_) => format!(
            "{}. The unfinished upload (a hidden `.chariox-copy-*` file beside `{destination}`) may remain on the leased agent's kernel; the next copy to that path replaces it",
            reason.trim_end_matches('.')
        ),
    }
}

fn admit(source: &str, size: u64, sha256: &str, options: &CopyOptions) -> Result<(), String> {
    if size > MAX_FILE_BYTES {
        return Err(format!(
            "`{source}` has {size} bytes; a copy moves at most {} MiB",
            MAX_FILE_BYTES / (1024 * 1024)
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

async fn read_local(workspace: &Workspace, path: &str, offset: u64) -> Result<SourceSlice, String> {
    let (workspace, path) = (workspace.clone(), path.to_owned());
    tokio::task::spawn_blocking(move || read_slice(&workspace, &path, offset))
        .await
        .map_err(|_| "The copy stopped unexpectedly; nothing was written".to_owned())?
}

/// Writes one pulled slice into the caller's workspace under the same rules a worker applies;
/// the last one publishes the file.
async fn land(
    local: &Workspace,
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
async fn patiently<T, F, Fut>(request: F) -> Result<T, String>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, LinkError>>,
{
    wait_for_link(RESUME_WINDOW, request).await
}

async fn wait_for_link<T, F, Fut>(window: Duration, request: F) -> Result<T, String>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, LinkError>>,
{
    let deadline = tokio::time::Instant::now() + window;
    let mut attempt = 0;
    loop {
        match request().await {
            Ok(value) => return Ok(value),
            Err(LinkError::Refused(reason)) => return Err(reason),
            Err(LinkError::Offline(why)) => {
                let delay = RETRY_DELAYS[attempt.min(RETRY_DELAYS.len() - 1)];
                if tokio::time::Instant::now() + delay > deadline {
                    return Err(format!(
                        "The leased agent's kernel stayed unreachable for {} s ({why}); nothing was written. Copy again once it is back",
                        window.as_secs()
                    ));
                }
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
    }
}
