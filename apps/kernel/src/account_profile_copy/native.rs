//! Native Codex credentials are checked in an isolated staging profile before publication.
use super::*;

struct CopyStage(PathBuf);

impl Drop for CopyStage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn verify_codex_copy(
    root: &Path,
    credential: &ProviderAccountCopyCredential,
) -> Result<zeroize::Zeroizing<String>, DaemonError> {
    fn failed<T>(_: T) -> DaemonError {
        failure("provider_account_copy_verification_failed")
    }
    let parent = root
        .parent()
        .ok_or_else(|| failure("provider_account_copy_install_failed"))?;
    fs::create_dir_all(parent).map_err(failed)?;
    set_private_dir_permissions(parent).map_err(failed)?;
    let stage = parent.join(format!(".account-copy-{:016x}", rand::random::<u64>()));
    fs::create_dir(&stage).map_err(failed)?;
    let stage = CopyStage(stage);
    set_private_dir_permissions(&stage.0).map_err(failed)?;
    let locator = ProviderAccountLocator::managed("codex", &stage.0).map_err(failed)?;
    create_private_roots(&locator).map_err(failed)?;
    let ProviderAccountLocator::Codex { codex_home } = &locator else {
        unreachable!()
    };
    enforce_codex_file_credentials(codex_home).map_err(failed)?;
    atomic_write_private(&codex_home.join("auth.json"), credential.value().as_bytes())
        .map_err(failed)?;
    let executable = crate::provider::resolve_codex_executable().map_err(failed)?;
    let mut command = crate::provider::managed_isolated_utility_command(
        executable.display().to_string(),
        vec!["login".into(), "status".into()],
        locator.environment(),
        None,
        "codex:account-copy",
    )
    .map_err(failed)?;
    for name in provider_auth_env_vars("codex") {
        command.env_remove(name);
    }
    let output = crate::runtime::bounded_process::capture::capture(
        command,
        std::time::Duration::from_secs(60),
    )
    .map_err(failed)?;
    if !output.status.success() {
        return Err(failure(if output.status.code() == Some(1) {
            "provider_account_copy_rejected"
        } else {
            "provider_account_copy_verification_failed"
        }));
    }
    let auth =
        read_bounded_regular_file_no_follow(&codex_home.join("auth.json"), 16 * 1024, "auth.json")
            .map_err(failed)?
            .ok_or_else(|| failure("provider_account_copy_rejected"))?;
    String::from_utf8(auth)
        .map(zeroize::Zeroizing::new)
        .map_err(failed)
}
