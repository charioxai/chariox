//! The worker's Seatbelt profile: the static broker-only policy, plus file
//! grants beneath an installed App's real `data` and `tmp` directories. Paths
//! travel as profile parameters, never as SBPL text, so no host path can
//! change the policy. Pure text so Linux CI checks exactly what macOS enters.
use std::{
    ffi::CString,
    io,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

const BASE: &str = include_str!("macos.sb");

pub(super) struct Profile {
    pub(super) text: CString,
    /// Alternating parameter names and values, as `sandbox_init_with_parameters`
    /// reads them; the caller appends the terminating null pointer.
    pub(super) parameters: Vec<CString>,
}

/// `files` must be canonical: Seatbelt matches the resolved path of every
/// operation, so a symlinked spelling would grant nothing.
pub(super) fn profile(files: &[PathBuf]) -> io::Result<Profile> {
    let mut text = BASE.to_owned();
    let mut parameters = Vec::new();
    if !files.is_empty() {
        let mut parents: Vec<&Path> = Vec::new();
        // File contents and directory entries only. Mode, owner, flags,
        // timestamps, extended attributes and links stay refused, as Linux
        // seccomp refuses them; sockets, exec and devices have no grant at all.
        text.push_str(
            "; An installed App's own files: read/write beneath data and tmp only.\n\
             (allow file-read* file-write-create file-write-data file-write-unlink",
        );
        for (index, file) in files.iter().enumerate() {
            if !file.is_absolute() || file.parent().is_none() {
                return Err(invalid("App file grants need absolute directories"));
            }
            let name = format!("APP_FILES_{index}");
            text.push_str(&format!("\n    (subpath (param \"{name}\"))"));
            parameters.extend([parameter(&name)?, parameter(file)?]);
            for parent in file.ancestors().skip(1) {
                if !parents.contains(&parent) {
                    parents.push(parent);
                }
            }
        }
        // Reopening a preopen resolves its path (realpath lstat's each
        // component). Metadata of those exact directories, nothing beneath.
        text.push_str(")\n(allow file-read-metadata");
        for (index, parent) in parents.into_iter().enumerate() {
            let name = format!("APP_PARENT_{index}");
            text.push_str(&format!("\n    (literal (param \"{name}\"))"));
            parameters.extend([parameter(&name)?, parameter(parent)?]);
        }
        text.push_str(")\n");
    }
    Ok(Profile {
        text: CString::new(text).map_err(|_| invalid("profile text"))?,
        parameters,
    })
}

fn parameter(value: impl AsRef<std::ffi::OsStr>) -> io::Result<CString> {
    CString::new(value.as_ref().as_bytes()).map_err(|_| invalid("App file path contains NUL"))
}

fn invalid(reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("APP_SANDBOX_FILES: {reason}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(profile: &Profile) -> &str {
        profile.text.to_str().unwrap()
    }
    fn parameters(profile: &Profile) -> Vec<(&str, &str)> {
        profile
            .parameters
            .chunks(2)
            .map(|pair| (pair[0].to_str().unwrap(), pair[1].to_str().unwrap()))
            .collect()
    }

    #[test]
    fn base_policy_grants_no_file_network_or_process_authority() {
        let profile = profile(&[]).unwrap();
        assert_eq!(text(&profile), BASE);
        assert!(profile.parameters.is_empty());
        assert!(BASE.contains("(deny default)"));
        for authority in [
            "(allow file",
            "(allow network",
            "(allow process-exec",
            "(allow process-fork",
            "(allow mach-lookup",
            "(allow ipc",
        ] {
            assert!(!BASE.contains(authority), "base grants {authority}");
        }
    }

    #[test]
    fn installed_files_grant_read_write_beneath_data_and_tmp_only() {
        let root = "/Users/o\"wner)/Library/Chariox/apps/app_1";
        let files = [format!("{root}/data"), format!("{root}/tmp")].map(PathBuf::from);
        let profile = profile(&files).unwrap();
        let grants = text(&profile).strip_prefix(BASE).unwrap();
        let parents = (0..7)
            .map(|index| format!("\n    (literal (param \"APP_PARENT_{index}\"))"))
            .collect::<String>();
        assert_eq!(
            grants,
            format!(
                "; An installed App's own files: read/write beneath data and tmp only.\n\
                 (allow file-read* file-write-create file-write-data file-write-unlink\n    \
                 (subpath (param \"APP_FILES_0\"))\n    (subpath (param \"APP_FILES_1\")))\n\
                 (allow file-read-metadata{parents})\n"
            )
        );
        // Host paths never become policy text, even with SBPL metacharacters.
        assert!(!text(&profile).contains("/Users"));
        assert_eq!(
            parameters(&profile),
            [
                ("APP_FILES_0", format!("{root}/data").as_str()),
                ("APP_FILES_1", &format!("{root}/tmp")),
                ("APP_PARENT_0", root),
                ("APP_PARENT_1", "/Users/o\"wner)/Library/Chariox/apps"),
                ("APP_PARENT_2", "/Users/o\"wner)/Library/Chariox"),
                ("APP_PARENT_3", "/Users/o\"wner)/Library"),
                ("APP_PARENT_4", "/Users/o\"wner)"),
                ("APP_PARENT_5", "/Users"),
                ("APP_PARENT_6", "/"),
            ]
        );
    }

    #[test]
    fn relative_root_or_nul_paths_are_refused() {
        for files in [
            vec![PathBuf::from("data")],
            vec![PathBuf::from("/")],
            vec![PathBuf::from(std::ffi::OsStr::from_bytes(
                b"/apps/a\0b/data",
            ))],
        ] {
            let error = profile(&files).err().unwrap();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(error.to_string().starts_with("APP_SANDBOX_FILES: "));
        }
    }
}
