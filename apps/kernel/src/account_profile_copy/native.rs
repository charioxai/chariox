//! Native Codex and OpenCode credentials are checked in an isolated staging profile before publication.
use super::*;

struct CopyStage(PathBuf);

impl Drop for CopyStage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn failed<T>(_: T) -> DaemonError {
    failure("provider_account_copy_verification_failed")
}

/// A receiver without the provider's CLI cannot check the login; say so rather
/// than "verification did not complete".
pub(super) fn cli_missing_or_failed(error: DaemonError) -> DaemonError {
    if matches!(error, DaemonError::ProviderExecutableNotFound { .. }) {
        failure("provider_account_copy_cli_missing")
    } else {
        failed(error)
    }
}

/// A private profile next to the destination, removed when the check ends.
fn staging_profile(
    root: &Path,
    provider: &str,
) -> Result<(CopyStage, ProviderAccountLocator), DaemonError> {
    let parent = root
        .parent()
        .ok_or_else(|| failure("provider_account_copy_install_failed"))?;
    fs::create_dir_all(parent).map_err(failed)?;
    set_private_dir_permissions(parent).map_err(failed)?;
    let stage = parent.join(format!(".account-copy-{:016x}", rand::random::<u64>()));
    fs::create_dir(&stage).map_err(failed)?;
    let stage = CopyStage(stage);
    set_private_dir_permissions(&stage.0).map_err(failed)?;
    let locator = ProviderAccountLocator::managed(provider, &stage.0).map_err(failed)?;
    create_private_roots(&locator).map_err(failed)?;
    Ok((stage, locator))
}

pub(super) fn verify_codex_copy(
    root: &Path,
    credential: &ProviderAccountCopyCredential,
) -> Result<zeroize::Zeroizing<String>, DaemonError> {
    let (_stage, locator) = staging_profile(root, "codex")?;
    let ProviderAccountLocator::Codex { codex_home } = &locator else {
        unreachable!()
    };
    enforce_codex_file_credentials(codex_home).map_err(failed)?;
    atomic_write_private(&codex_home.join("auth.json"), credential.value().as_bytes())
        .map_err(failed)?;
    let executable = crate::provider::resolve_codex_executable().map_err(cli_missing_or_failed)?;
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

pub(super) fn verify_opencode_copy(
    root: &Path,
    credential: &ProviderAccountCopyCredential,
) -> Result<zeroize::Zeroizing<String>, DaemonError> {
    use crate::local::provider_requests::OpenCodeCopyCheck;
    let (_stage, locator) = staging_profile(root, "opencode")?;
    let destination =
        materialization_destination(&locator, OPENCODE_COPY_CREDENTIAL_PATH).map_err(failed)?;
    atomic_write_private(&destination, credential.value().as_bytes()).map_err(failed)?;
    crate::local::provider_requests::check_copied_opencode_login(&locator.environment()).map_err(
        |check| match check {
            OpenCodeCopyCheck::CliMissing => failure("provider_account_copy_cli_missing"),
            OpenCodeCopyCheck::Rejected => failure("provider_account_copy_rejected"),
            OpenCodeCopyCheck::Inconclusive => failure("provider_account_copy_verification_failed"),
        },
    )?;
    Ok(zeroize::Zeroizing::new(credential.value().to_string()))
}
