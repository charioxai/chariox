//! MP-07 / MP-08 / MP-11: owner admission, approved releases and SSH deployment.
use crate::local::{LocalDaemonRequest, LocalDaemonResponse, SshMachineResult};
use crate::runtime::command::KernelCommand;
use crate::{DaemonConfig, DaemonError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Release {
    id: String,
    release_digest: String,
    archive: PathBuf,
    release_public_key: PathBuf,
    release_public_key_fingerprint: String,
    builder_public_key: PathBuf,
    builder_public_key_fingerprint: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Catalog {
    default_release: String,
    releases: Vec<Release>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Install {
    host: String,
    install_id: String,
    port: u16,
    release: Release,
}
#[derive(Deserialize)]
struct Ticket {
    ticket: String,
    ticket_id: String,
    expires_at: String,
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.ticket.zeroize();
    }
}
fn error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "SSH machine",
        message: message.into(),
    }
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, DaemonError> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|_| error("SSH machine catalogue/registry unavailable"))?;
    if !meta.is_file() || meta.len() > 65536 {
        return Err(error(
            "bounded regular SSH machine catalogue/registry required",
        ));
    }
    serde_json::from_slice(
        &std::fs::read(path).map_err(|_| error("SSH machine catalogue/registry unreadable"))?,
    )
    .map_err(|_| error("invalid SSH machine catalogue/registry"))
}
fn store(path: &Path, installs: &[Install]) -> Result<(), DaemonError> {
    crate::config::write_private_file(
        path,
        &serde_json::to_vec(installs).map_err(|_| error("invalid registry"))?,
    )
    .map_err(|_| error("cannot persist SSH install registry"))
}
fn validate(install: &Install) -> Result<(), DaemonError> {
    if install.host.is_empty()
        || install.host.len() > 255
        || !install.host.as_bytes()[0].is_ascii_alphanumeric()
        || !install
            .host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.@:[-]".contains(&b))
        || install.install_id.is_empty()
        || install.install_id.len() > 48
        || !install.install_id.as_bytes()[0].is_ascii_lowercase()
        || !install
            .install_id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || !install
            .release
            .release_digest
            .strip_prefix("sha256:")
            .is_some_and(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        || install.port < 1024
        || [43117, 43118, 43119, 43120, 65535].contains(&install.port)
    {
        return Err(error(
            "invalid SSH host, install ID or distinct loopback port",
        ));
    }
    Ok(())
}
// Cloud control only; no runtime payloads. Never echo Cloud errors or response bodies.
async fn ticket_request(
    profile: &crate::config::PersistedCloudRelayProfile,
    label: &str,
) -> Result<Ticket, DaemonError> {
    crate::owner_managed_bootstrap::admit_api_url(&profile.api_url)?;
    let profile = profile.clone();
    let label = label.to_string();
    tokio::task::spawn_blocking(move || {
        let credential = profile.kernel_credential.as_deref().filter(|s| !s.is_empty()).ok_or_else(|| error("source kernel must be independently enrolled"))?;
        let response = ureq::AgentBuilder::new().redirects(0).timeout(std::time::Duration::from_secs(30)).build()
            .post(&format!("{}/v1/kernel-enrollment-tickets", profile.api_url.trim_end_matches('/')))
            .set("x-chariox-kernel-credential", credential)
            .set("content-type", "application/json")
            .send_string(&json!({"purpose":"owner_managed_machine", "machine_label": label, "ttl_seconds":600}).to_string())
            .map_err(|_| error("Cloud enrollment ticket issue refused"))?;
        let bytes = { use std::io::Read; let mut bytes = Zeroizing::new(Vec::new()); response.into_reader().take(8193).read_to_end(&mut bytes).map_err(|_| error("ticket response unreadable"))?; bytes };
        // Parse the non-secret ID independently: even malformed ticket fields
        // must not leave an issued ticket active when this add is refused.
        #[derive(Deserialize)]
        struct IssuedId { ticket_id: String }
        let issued_id = serde_json::from_slice::<IssuedId>(&bytes).ok()
            .map(|value| value.ticket_id).filter(|id| !id.is_empty() && id.len() <= 255);
        let result = (|| {
            if bytes.len() > 8192 { return Err(error("ticket response too large")); }
            let ticket: Ticket = serde_json::from_slice(&bytes).map_err(|_| error("invalid ticket response"))?;
            validate_ticket(&ticket, chrono::Utc::now())?;
            Ok(ticket)
        })();
        if result.is_err() {
            if let Some(id) = issued_id { revoke_blocking(&profile, &id)?; }
        }
        result
    }).await.map_err(|_| error("ticket issue task failed"))?
}
// MP-07/MP-08/MP-11: Cloud and source clocks may differ slightly.
fn validate_ticket(ticket: &Ticket, now: chrono::DateTime<chrono::Utc>) -> Result<(), DaemonError> {
    let expiry = chrono::DateTime::parse_from_rfc3339(&ticket.expires_at)
        .map_err(|_| error("invalid ticket expiry"))?;
    let ttl = expiry.signed_duration_since(now);
    if ticket.ticket.is_empty()
        || ticket.ticket.len() > 4096
        || ticket.ticket_id.is_empty()
        || ticket.ticket_id.len() > 255
        || ttl <= chrono::Duration::zero()
        || ttl > chrono::Duration::seconds(603)
    {
        return Err(error("invalid ticket response"));
    }
    Ok(())
}
async fn revoke(
    profile: &crate::config::PersistedCloudRelayProfile,
    id: &str,
) -> Result<(), DaemonError> {
    let profile = profile.clone();
    let id = id.to_string();
    tokio::task::spawn_blocking(move || revoke_blocking(&profile, &id))
        .await
        .map_err(|_| error("ticket revocation task failed"))?
}
fn revoke_blocking(
    profile: &crate::config::PersistedCloudRelayProfile,
    id: &str,
) -> Result<(), DaemonError> {
    crate::owner_managed_bootstrap::admit_api_url(&profile.api_url)?;
    let credential = profile
        .kernel_credential
        .as_deref()
        .ok_or_else(|| error("missing kernel credential"))?;
    let response = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .delete(&format!(
            "{}/v1/kernel-enrollment-tickets/{}",
            profile.api_url.trim_end_matches('/'),
            crate::runtime::cloud_api_client::cloud_url_component(id)
        ))
        .set("x-chariox-kernel-credential", credential)
        .call();
    match response {
        Ok(r) if matches!(r.status(), 200 | 204) => Ok(()),
        Err(ureq::Error::Status(404, _)) => Ok(()),
        _ => Err(error(
            "Cloud ticket revocation failed; check the ticket in Cloud before retrying",
        )),
    }
}
const ASSETS: &[(&str, &str)] = &[
    (
        "apps/kernel/ssh-machine/driver.mjs",
        include_str!("../../ssh-machine/driver.mjs"),
    ),
    (
        "apps/kernel/ssh-machine/transport.mjs",
        include_str!("../../ssh-machine/transport.mjs"),
    ),
    (
        "apps/kernel/ssh-machine/upgrade.mjs",
        include_str!("../../ssh-machine/upgrade.mjs"),
    ),
    (
        "apps/kernel/ssh-machine/remote.mjs",
        include_str!("../../ssh-machine/remote.mjs"),
    ),
    (
        "deploy/managed-kernel/extract-release.py",
        include_str!("../../../../deploy/managed-kernel/extract-release.py"),
    ),
    (
        "deploy/managed-kernel/verify-image-release.mjs",
        include_str!("../../../../deploy/managed-kernel/verify-image-release.mjs"),
    ),
    (
        "deploy/managed-kernel/path1-service-policy.mjs",
        include_str!("../../../../deploy/managed-kernel/path1-service-policy.mjs"),
    ),
];
struct Scratch(PathBuf);
fn install_failure(status: Option<i32>, install_id: Option<&str>) -> DaemonError {
    if let Some(id) = install_id.filter(|id| {
        !id.is_empty()
            && id.len() <= 48
            && id.as_bytes()[0].is_ascii_lowercase()
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    }) {
        if status == Some(75) {
            return error(&format!("Another install operation owns this install ID. On the SSH target, check ~/.local/share/chariox/ssh-machines/.{id}.lock. Wait for the operation to finish; if interrupted, confirm no installer for this ID is running, remove only that empty lock directory, then retry the same command."));
        }
    }
    error("SSH install failed; check access, signed release, enrollment and user service prerequisites")
}
// MP-07/MP-08/MP-11: only home-validated request fields enter diagnostics.
fn deployment_failure(status: Option<i32>, input: &Value) -> DaemonError {
    let id = input.pointer("/request/installId").and_then(Value::as_str);
    let host = input["host"].as_str().unwrap_or("SSH target");
    let port = input.pointer("/request/port").and_then(Value::as_u64);
    match (status, id, port) {
        (Some(76 | 78), Some(_), Some(port)) => {
            let port = port + u64::from(status == Some(78));
            error(&format!("port {port} is in use on {host}; choose another --port (both the kernel port and the next MCP port must be free)"))
        }
        (Some(77), Some(id), _) => error(&format!("unit chariox-ssh-{id}.service on {host} belongs to another installation; choose another --id or ask the host administrator to resolve the foreign unit")),
        _ => install_failure(status, id),
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn deploy(state: &Path, input: Value) -> Result<Value, DaemonError> {
    let scratch = Scratch(state.join(format!("ssh-operation-{:016x}", rand::random::<u64>())));
    std::fs::create_dir(&scratch.0).map_err(|_| error("cannot create SSH operation scratch"))?;
    for (name, bytes) in ASSETS {
        let path = scratch.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap())
            .map_err(|_| error("cannot create SSH helper path"))?;
        std::fs::write(path, bytes).map_err(|_| error("cannot materialize SSH helper"))?;
    }
    let bytes =
        Zeroizing::new(serde_json::to_vec(&input).map_err(|_| error("invalid SSH operation"))?);
    let mut child = tokio::process::Command::new("node")
        .arg(scratch.0.join("apps/kernel/ssh-machine/driver.mjs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| error("Node 22 and user SSH are required"))?;
    if child.id().is_none_or(|pid| pid <= 1) {
        return Err(error("unsafe deployment process identity"));
    }
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| error("SSH stdin unavailable"))?;
    stdin
        .write_all(&bytes)
        .await
        .map_err(|_| error("SSH stdin failed"))?;
    drop(stdin);
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(570),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| error("SSH install timed out"))?
    .map_err(|_| error("SSH install subprocess failed"))?;
    if !output.status.success() || output.stdout.len() > 8192 {
        return Err(deployment_failure(output.status.code(), &input));
    }
    serde_json::from_slice(&output.stdout).map_err(|_| error("invalid SSH install result"))
}

// MP-08 / MP-11: reconcile only a positively absent target. Unknown SSH failures retain selection.
async fn target_absent(state: &Path, install: &Install) -> bool {
    deploy(state, json!({"host":install.host,"request":{"action":"inspect","installId":install.install_id,"port":install.port,"releaseDigest":install.release.release_digest}})).await
        .is_ok_and(|result| result["installId"] == install.install_id && result["status"] == "absent")
}
pub(crate) async fn execute(
    config: DaemonConfig,
    command: &KernelCommand,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    if !crate::runtime::cloud_relay_authorization::kernel_cloud_owner(&config, command) {
        return Err(error(
            "only this kernel's owner may install or remove SSH machines",
        ));
    }
    let profile = config
        .cloud_relay
        .as_ref()
        .filter(|p| p.kernel_credential.is_some())
        .ok_or_else(|| error("independent source kernel enrollment required"))?;
    let state = config.private_runtime_state_root();
    std::fs::create_dir_all(&state).map_err(|_| error("cannot create kernel state"))?;
    let lock_path = state.join("ssh-machines.lock");
    let _lock = tokio::task::spawn_blocking(move || {
        use fs2::FileExt;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(|_| error("cannot open SSH registry lock"))?;
        file.lock_exclusive()
            .map_err(|_| error("cannot lock SSH registry"))?;
        Ok::<_, DaemonError>(file)
    })
    .await
    .map_err(|_| error("SSH registry lock failed"))??;
    let registry = state.join("ssh-machines.json");
    let mut installs: Vec<Install> = if registry.exists() {
        read(&registry)?
    } else {
        Vec::new()
    };
    let (install, action) = match request {
        LocalDaemonRequest::AddSshMachine(r) => {
            let catalog: Catalog = read(
                &config
                    .user_config_path()
                    .parent()
                    .ok_or_else(|| error("missing config root"))?
                    .join("ssh-machine-releases.json"),
            )?;
            let selected = r.release.as_ref().unwrap_or(&catalog.default_release);
            let release = catalog
                .releases
                .into_iter()
                .find(|r| &r.id == selected)
                .ok_or_else(|| {
                    error("chosen signed release is absent from the approved catalogue")
                })?;
            let digest = format!("{:x}", Sha256::digest(r.host.as_bytes()));
            let install = Install {
                host: r.host,
                install_id: r
                    .install_id
                    .unwrap_or_else(|| format!("ssh-{}", &digest[..12])),
                port: r
                    .port
                    .unwrap_or(55000 + u16::from_str_radix(&digest[..4], 16).unwrap() % 5000),
                release,
            };
            validate(&install)?;
            if let Some(old) = installs
                .iter()
                .find(|old| old.install_id == install.install_id)
                .cloned()
            {
                if old.host != install.host
                    || old.port != install.port
                    || old.release.release_digest != install.release.release_digest
                {
                    if !target_absent(&state, &old).await {
                        return Err(error("existing install differs; reconcile SSH ownership before an explicit upgrade"));
                    }
                    installs.retain(|item| item.install_id != old.install_id);
                    store(&registry, &installs)?;
                }
            }
            (install, "install")
        }
        LocalDaemonRequest::RemoveSshMachine(r) => (
            installs
                .iter()
                .find(|i| i.install_id == r.install_id)
                .cloned()
                .ok_or_else(|| error("no SSH installation with that ID"))?,
            "remove",
        ),
        _ => return Err(error("unsupported SSH machine operation")),
    };
    validate(&install)?;
    if action == "remove" && target_absent(&state, &install).await {
        installs.retain(|item| item.install_id != install.install_id);
        store(&registry, &installs)?;
        return Ok(LocalDaemonResponse::SshMachine {
            machine: SshMachineResult {
                install_id: install.install_id,
                status: "removed".into(),
                kernel_id: None,
                machine_id: None,
                release_digest: install.release.release_digest,
                state_retained: true,
            },
        });
    }
    let mut input = json!({"host":install.host,"request":{"action":action,"installId":install.install_id,"port":install.port,"releaseDigest":install.release.release_digest},"release":install.release});
    let ticket = if action == "install" {
        Some(ticket_request(profile, &install.install_id).await?)
    } else {
        None
    };
    // Ticket issue failure cannot publish anything remotely; do not reserve beforehand.
    if action == "install"
        && !installs
            .iter()
            .any(|old| old.install_id == install.install_id)
    {
        installs.push(install.clone());
        if let Err(store_error) = store(&registry, &installs) {
            if let Some(ticket) = &ticket {
                revoke(profile, &ticket.ticket_id).await?;
            }
            return Err(store_error);
        }
    }
    if let Some(ticket) = &ticket {
        input["enrollment"] =
            json!({"ticket":ticket.ticket,"apiUrl":profile.api_url,"userId":profile.user_id});
    }
    let result = deploy(&state, input).await;
    // Successful redemption consumes the ticket. Revoke only failures or an
    // idempotent target that retained its already enrolled identity.
    if !result
        .as_ref()
        .is_ok_and(|value| value["ticketConsumed"] == true)
    {
        if let Some(ticket) = &ticket {
            revoke(profile, &ticket.ticket_id).await?;
        }
    }
    if result.is_err() && action == "install" && target_absent(&state, &install).await {
        installs.retain(|item| item.install_id != install.install_id);
        store(&registry, &installs)?;
    }
    let result = result?;
    let status = if action == "install" {
        "ready"
    } else {
        "removed"
    };
    if result["installId"] != install.install_id || result["status"] != status {
        return Err(error("SSH install result does not match its request"));
    }
    if action == "remove" {
        installs.retain(|i| i.install_id != install.install_id);
        store(&registry, &installs)?;
    }
    Ok(LocalDaemonResponse::SshMachine {
        machine: SshMachineResult {
            install_id: install.install_id,
            status: status.into(),
            kernel_id: result["kernelId"].as_str().map(String::from),
            machine_id: result["machineId"].as_str().map(String::from),
            release_digest: install.release.release_digest,
            state_retained: action == "remove",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byom_mp08_mp11_ticket_accepts_bounded_clock_skew_without_extending_expired_tickets() {
        let cloud = chrono::DateTime::parse_from_rfc3339("2026-10-12T08:23:57.024Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let ticket = Ticket {
            ticket: "synthetic-single-use".into(),
            ticket_id: "ticket-id".into(),
            expires_at: (cloud + chrono::Duration::seconds(600)).to_rfc3339(),
        };
        for skew_ms in [50, 100, 200, 3000] {
            assert!(
                validate_ticket(&ticket, cloud - chrono::Duration::milliseconds(skew_ms)).is_ok(),
                "valid ticket refused at {skew_ms}ms skew"
            );
        }
        assert!(validate_ticket(&ticket, cloud - chrono::Duration::seconds(4)).is_err());
        assert!(validate_ticket(&ticket, cloud + chrono::Duration::seconds(600)).is_err());
    }
    #[test]
    fn byom_mp07_mp11_lock_exit_maps_only_a_validated_install_id_to_recovery() {
        let message = install_failure(Some(75), Some("byom-one")).to_string();
        assert!(message.contains("~/.local/share/chariox/ssh-machines/.byom-one.lock"));
        assert!(message.contains("confirm no installer"));
        for id in [None, Some(""), Some("../private"), Some("a\nsecret")] {
            assert!(!install_failure(Some(75), id).to_string().contains(".lock"));
        }
        for status in [None, Some(1), Some(255)] {
            assert!(!install_failure(status, Some("byom-one"))
                .to_string()
                .contains(".lock"));
        }
    }
    #[tokio::test]
    async fn byom_mp08_mp11_issue_revoke_contract_uses_kernel_header_and_no_human_session() {
        ticket_http_fixture(json!({"ticket":"fixture-single-use", "ticket_id":"ticket-id", "expires_at":(chrono::Utc::now() + chrono::Duration::seconds(590)).to_rfc3339()}), false).await;
    }
    #[tokio::test]
    async fn byom_mp08_mp11_refused_ticket_is_revoked_even_when_only_id_parses() {
        for body in [
            json!({"ticket":"fixture-single-use", "ticket_id":"ticket-id", "expires_at":(chrono::Utc::now() - chrono::Duration::seconds(1)).to_rfc3339()}),
            json!({"ticket":"fixture-single-use", "ticket_id":"ticket-id", "expires_at":"invalid"}),
            json!({"ticket_id":"ticket-id", "expires_at":"invalid"}),
        ] {
            ticket_http_fixture(body, true).await;
        }
    }
    async fn ticket_http_fixture(body: Value, refused: bool) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let fixture = std::thread::spawn(move || {
            let mut routes = Vec::new();
            for index in 0..2 {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && std::time::Instant::now() < deadline =>
                        {
                            std::thread::sleep(std::time::Duration::from_millis(5))
                        }
                        Err(_) => panic!("ticket issue/refusal must reach revocation"),
                    }
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = stream.read(&mut chunk).unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                    let request = String::from_utf8_lossy(&bytes);
                    if let Some(boundary) = request.find("\r\n\r\n") {
                        let length = request[..boundary]
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .and_then(|v| v.parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= boundary + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8(bytes).unwrap();
                assert!(request
                    .to_ascii_lowercase()
                    .contains("x-chariox-kernel-credential: fixture-kernel-authority"));
                assert!(!request.contains("authorization:"));
                routes.push(request.lines().next().unwrap().to_string());
                let body = if index == 0 {
                    let sent: Value =
                        serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
                    assert_eq!(
                        sent,
                        json!({"purpose":"owner_managed_machine","machine_label":"owned-machine","ttl_seconds":600})
                    );
                    body.to_string()
                } else {
                    "{}".into()
                };
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            }
            routes
        });
        let profile = crate::config::PersistedCloudRelayProfile {
            api_url,
            kernel_credential: Some("fixture-kernel-authority".into()),
            cloud_session_token: Some("must-not-be-used".into()),
            ..Default::default()
        };
        let result = ticket_request(&profile, "owned-machine").await;
        if refused {
            assert!(result.is_err());
        } else {
            let ticket = result.unwrap();
            assert!(!ticket.ticket.is_empty());
            revoke(&profile, &ticket.ticket_id).await.unwrap();
        }
        assert_eq!(
            fixture.join().unwrap(),
            vec![
                "POST /v1/kernel-enrollment-tickets HTTP/1.1",
                "DELETE /v1/kernel-enrollment-tickets/ticket-id HTTP/1.1"
            ]
        );
    }
    #[test]
    fn byom_mp07_mp08_mp11_conflicts_name_the_target_and_fix() {
        let input = json!({"host":"byom-target", "request":{"installId":"byom-one", "port":47310}});
        for (code, port) in [(76, 47310), (78, 47311)] {
            let message = deployment_failure(Some(code), &input).to_string();
            assert!(message.contains(&format!("port {port} is in use on byom-target")));
            assert!(message.contains("choose another --port"));
        }
        let message = deployment_failure(Some(77), &input).to_string();
        assert!(message.contains("chariox-ssh-byom-one.service on byom-target"));
        assert!(message.contains("choose another --id"));
    }
    #[test]
    fn byom_mp11_catalog_selection_rejects_shell_hosts_default_port_and_foreign_ids() {
        let release: Release = serde_json::from_value(json!({"id":"approved","releaseDigest":format!("sha256:{}", "a".repeat(64)),"archive":"/fixture","releasePublicKey":"/public","releasePublicKeyFingerprint":"sha256:public","builderPublicKey":"/builder","builderPublicKeyFingerprint":"sha256:builder"})).unwrap();
        let mut install = Install {
            host: "linux-lan".into(),
            install_id: "byom-lan".into(),
            port: 55129,
            release,
        };
        assert!(validate(&install).is_ok());
        install.host = "-oProxyCommand=evil".into();
        assert!(validate(&install).is_err());
        install.host = "linux-lan".into();
        for port in [43117, 43118, 43119, 43120, 65535] {
            install.port = port;
            assert!(validate(&install).is_err());
        }
        install.port = 55129;
        let digest = install.release.release_digest.clone();
        install.release.release_digest = "sha256:invalid".into();
        assert!(validate(&install).is_err());
        install.release.release_digest = digest;
        install.install_id = "../md-staging".into();
        assert!(validate(&install).is_err());
    }
}
