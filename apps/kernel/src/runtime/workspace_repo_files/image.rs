//! A picture file inside a workspace, read for a tool call's transcript entry.

use std::io::Read;
use std::path::Path;

use super::shared::workspace_file_root;

/// What a viewed file turned out to be.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WorkspacePicture {
    Picture {
        media_type: &'static str,
        bytes: Vec<u8>,
    },
    /// A PNG, JPEG, GIF or WebP file larger than the limit.
    TooLarge,
}

/// Reads `path` (absolute, or relative to `workspace`) when it is a PNG, JPEG, GIF or WebP file
/// inside the workspace's repository root, by the same containment rule as workspace file reads:
/// the resolved file must lie under the resolved root, so neither `..` nor a symlink leads out.
/// Anything else, including an unreadable file, is `None`.
pub(crate) fn read_workspace_picture(
    workspace: &Path,
    path: &Path,
    max_bytes: usize,
) -> Option<WorkspacePicture> {
    let (root, _) = workspace_file_root(workspace.to_str()?).ok()?;
    let root = std::fs::canonicalize(root).ok()?;
    let file = std::fs::canonicalize(workspace.join(path)).ok()?;
    if !file.starts_with(&root) || !file.is_file() {
        return None;
    }
    let mut file = std::fs::File::open(file).ok()?;
    let size = file.metadata().ok()?.len();
    let mut header = Vec::new();
    file.by_ref().take(12).read_to_end(&mut header).ok()?;
    let media_type = picture_media_type(&header)?;
    if size > max_bytes as u64 {
        return Some(WorkspacePicture::TooLarge);
    }
    let mut bytes = header;
    file.take(max_bytes as u64 + 1 - bytes.len() as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > max_bytes {
        return Some(WorkspacePicture::TooLarge);
    }
    Some(WorkspacePicture::Picture { media_type, bytes })
}

fn picture_media_type(header: &[u8]) -> Option<&'static str> {
    if header.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("image/png")
    } else if header.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}
