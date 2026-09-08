use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;

use crate::error::DaemonError;
use crate::session::PromptAttachment;

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
            let root = inline_prompt_attachment_root(session_id, agent_id);
            let mut directory = fs::DirBuilder::new();
            directory.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                directory.mode(0o700);
            }
            directory
                .create(&root)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "create inline prompt attachment directory",
                    message: error.to_string(),
                })?;
            let validate_error = |error: std::io::Error| DaemonError::LocalTransport {
                operation: "validate inline prompt attachment directory",
                message: error.to_string(),
            };
            let expected = std::env::temp_dir()
                .canonicalize()
                .map_err(validate_error)?
                .join(
                    root.strip_prefix(std::env::temp_dir())
                        .expect("attachment root is beneath temp directory"),
                );
            if root.canonicalize().map_err(validate_error)? != expected {
                return Err(DaemonError::LocalTransport {
                    operation: "validate inline prompt attachment directory",
                    message: "attachment directory must not traverse symbolic links".to_string(),
                });
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
                    .map_err(validate_error)?;
            }
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
    std::env::temp_dir()
        .join(INLINE_PROMPT_ATTACHMENT_DIR)
        .join(sanitize_path_component(session_id))
        .join(sanitize_path_component(agent_id))
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
    fn attachment_writer_rejects_symlink_directory_without_writing() {
        let fixture = Fixture::new();
        let target = fixture.root.parent().unwrap().join("target");
        fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, &fixture.root).unwrap();
        assert!(fixture.materialize().is_err());
        assert_eq!(fs::read_dir(target).unwrap().count(), 0);
    }
}
