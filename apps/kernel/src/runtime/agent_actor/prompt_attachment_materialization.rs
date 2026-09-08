use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;

use crate::error::DaemonError;
use crate::session::PromptAttachment;

#[cfg(unix)]
#[path = "prompt_attachment_directory.rs"]
mod private_directory;

pub(crate) const INLINE_PROMPT_ATTACHMENT_DIR: &str = "chariox-terminal-prompt-attachments";
static NEXT_ATTACHMENT_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) fn materialize_inline_prompt_attachments(
    session_id: &str,
    agent_id: &str,
    attachments: Vec<PromptAttachment>,
) -> Result<Vec<PromptAttachment>, DaemonError> {
    attachments
        .into_iter()
        .enumerate()
        .map(|(index, attachment)| {
            let Some(contents_base64) = attachment.contents_base64() else {
                return Ok(attachment);
            };
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(contents_base64)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "decode inline prompt attachment",
                    message: error.to_string(),
                })?;
            let filename = attachment
                .filename()
                .map(sanitize_attachment_filename)
                .unwrap_or_else(|| format!("attachment-{index}"));
            let root = prepare_inline_prompt_attachment_root(session_id, agent_id)?;
            let path = root.join(format!(
                "{}-{}-{}-{}-{}",
                crate::session::unix_epoch_ms(),
                std::process::id(),
                NEXT_ATTACHMENT_ID.fetch_add(1, Ordering::Relaxed),
                index,
                filename
            ));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options
                .open(&path)
                .and_then(|mut file| file.write_all(&bytes))
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "write inline prompt attachment",
                    message: error.to_string(),
                })?;
            Ok(PromptAttachment::new(
                format!("file://{}", path.display()),
                attachment.mime().to_string(),
                Some(filename),
            ))
        })
        .collect()
}

pub(crate) fn inline_prompt_attachment_root(session_id: &str, agent_id: &str) -> PathBuf {
    #[cfg(unix)]
    let namespace = format!(
        "{INLINE_PROMPT_ATTACHMENT_DIR}-{}",
        private_directory::current_uid()
    );
    #[cfg(not(unix))]
    let namespace = INLINE_PROMPT_ATTACHMENT_DIR;
    std::env::temp_dir()
        .join(namespace)
        .join(sanitize_path_component(session_id))
        .join(sanitize_path_component(agent_id))
}

pub(crate) fn prepare_inline_prompt_attachment_root(
    session_id: &str,
    agent_id: &str,
) -> Result<PathBuf, DaemonError> {
    let root = inline_prompt_attachment_root(session_id, agent_id);
    #[cfg(unix)]
    let prepared = private_directory::prepare(&root, &std::env::temp_dir());
    #[cfg(not(unix))]
    let prepared = prepare_nonunix_directory(&root);
    prepared.map_err(|error| DaemonError::LocalTransport {
        operation: "prepare private inline prompt attachment directory",
        message: error.to_string(),
    })?;
    Ok(root)
}

#[cfg(not(unix))]
fn prepare_nonunix_directory(root: &Path) -> std::io::Result<()> {
    let temp = std::env::temp_dir();
    let relative = root
        .strip_prefix(&temp)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    fs::create_dir_all(root)?;
    if root.canonicalize()? != temp.canonicalize()?.join(relative) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "attachment directory must not traverse symbolic links",
        ));
    }
    Ok(())
}

fn sanitize_attachment_filename(value: &str) -> String {
    let file_name = Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment");
    sanitize_path_component(file_name)
}

fn sanitize_path_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches(['.', '-']);
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(all(test, unix))]
mod permission_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture {
        session: String,
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let session = format!(
                "attachment-permissions-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let root = inline_prompt_attachment_root(&session, "agent");
            Self { session, root }
        }
        fn materialize(&self) -> Result<Vec<PromptAttachment>, DaemonError> {
            materialize_inline_prompt_attachments(
                &self.session,
                "agent",
                vec![PromptAttachment::new(
                    "synthetic://permissions",
                    "text/plain",
                    Some("probe.txt".into()),
                )
                .with_contents_base64("cHJpdmF0ZQ==")],
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(self.root.parent().unwrap());
        }
    }

    #[test]
    fn empty_attachment_root_is_private_before_materialization() {
        let fixture = Fixture::new();
        let root = prepare_inline_prompt_attachment_root(&fixture.session, "agent").unwrap();
        assert_eq!(root, fixture.root);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        for path in [
            &root,
            root.parent().unwrap(),
            root.parent().unwrap().parent().unwrap(),
        ] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn attachment_writer_makes_existing_directory_and_new_file_private() {
        let fixture = Fixture::new();
        fs::create_dir_all(&fixture.root).unwrap();
        fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o755)).unwrap();
        fixture.materialize().unwrap();
        assert_eq!(
            fs::metadata(&fixture.root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let path = fs::read_dir(&fixture.root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read(&path).unwrap(), b"private");
    }

    #[test]
    fn attachment_writer_makes_session_directory_private() {
        let fixture = Fixture::new();
        let parent = fixture.root.parent().unwrap();
        fs::create_dir_all(parent).unwrap();
        fs::set_permissions(parent, fs::Permissions::from_mode(0o755)).unwrap();
        fixture.materialize().unwrap();
        assert_eq!(
            fs::metadata(parent).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn attachment_writer_rejects_symlink_parent_without_creating_children() {
        let fixture = Fixture::new();
        let target = Fixture::new();
        fs::create_dir_all(&target.root).unwrap();
        std::os::unix::fs::symlink(&target.root, fixture.root.parent().unwrap()).unwrap();
        assert!(fixture.materialize().is_err());
        assert_eq!(fs::read_dir(&target.root).unwrap().count(), 0);
    }

    #[test]
    fn attachment_writer_rejects_symlink_directory_without_writing() {
        let fixture = Fixture::new();
        let target = fixture.root.parent().unwrap().join("target");
        fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, &fixture.root).unwrap();
        assert!(fixture.materialize().is_err());
        assert_eq!(fs::read_dir(target).unwrap().count(), 0);
    }
}
