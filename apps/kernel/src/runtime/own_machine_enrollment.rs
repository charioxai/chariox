//! A single-use Cloud ticket, wrapped in the command that enrolls a new own machine.
//! The command is the only place the ticket appears; it is never logged or stored.
use crate::local::{LocalDaemonResponse, OwnMachineEnrollmentCommand};
use crate::runtime::command::KernelCommand;
use crate::{DaemonConfig, DaemonError};
use serde::Deserialize;
use serde_json::json;
use std::io::Read;
use zeroize::{Zeroize, Zeroizing};

const TICKET_SECONDS: i64 = 600;
// Cloud and kernel clocks may differ slightly.
const CLOCK_SKEW_SECONDS: i64 = 3;

fn error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "own machine enrollment",
        message: message.into(),
    }
}

#[derive(Deserialize)]
struct Ticket {
    ticket: String,
    ticket_id: String,
    expires_at: String,
    setup_install_url: Option<String>,
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.ticket.zeroize();
    }
}

#[derive(Deserialize)]
struct IssuedId {
    ticket_id: String,
}

/// Only the kernel's owner may enroll machines into the owner's account, and a request
/// that names a user must name that owner: a guest's agent runs with the guest's identity.
fn owner_profile<'a>(
    config: &'a DaemonConfig,
    command: &KernelCommand,
) -> Result<&'a crate::config::PersistedCloudRelayProfile, DaemonError> {
    let profile = crate::runtime::cloud_relay_control::active_cloud_profile(config)
        .filter(|p| {
            p.kernel_credential
                .as_deref()
                .is_some_and(|c| !c.is_empty())
        })
        .ok_or_else(|| {
            error(
                "this kernel is not signed in to Chariox; sign in before enrolling another machine",
            )
        })?;
    let named_other = command
        .caller
        .user_id
        .as_deref()
        .is_some_and(|user| user != profile.user_id);
    if !crate::runtime::cloud_relay_authorization::kernel_cloud_owner(config, command)
        || named_other
    {
        return Err(error("only this kernel's owner may enroll a machine"));
    }
    Ok(profile)
}

fn ticket_request_body() -> serde_json::Value {
    json!({"purpose":"owner_managed_machine","ttl_seconds":TICKET_SECONDS})
}

fn issue_blocking(
    profile: &crate::config::PersistedCloudRelayProfile,
) -> Result<Ticket, DaemonError> {
    crate::owner_managed_bootstrap::admit_api_url(&profile.api_url)?;
    let credential = profile.kernel_credential.as_deref().unwrap_or_default();
    let response = crate::outbound_dial::agent_builder()
        .redirects(0)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .post(&format!("{}/v1/kernel-enrollment-tickets", profile.api_url.trim_end_matches('/')))
        .set("x-chariox-kernel-credential", credential)
        .set("content-type", "application/json")
        .send_string(&ticket_request_body().to_string())
        .map_err(|_| error("Chariox refused to issue an enrollment ticket; check this kernel's sign-in and retry"))?;
    let mut bytes = Zeroizing::new(Vec::new());
    response
        .into_reader()
        .take(8193)
        .read_to_end(&mut bytes)
        .map_err(|_| error("the enrollment ticket response was unreadable"))?;
    if bytes.len() > 8192 {
        return Err(error("the enrollment ticket response was too large"));
    }
    // Parse the public ID on its own: a malformed ticket must still be revoked.
    let issued = serde_json::from_slice::<IssuedId>(&bytes)
        .ok()
        .map(|v| v.ticket_id)
        .filter(|id| !id.is_empty() && id.len() <= 255);
    match serde_json::from_slice::<Ticket>(&bytes) {
        Ok(ticket) => Ok(ticket),
        Err(_) => {
            if let Some(id) = issued {
                revoke_blocking(profile, &id)?;
            }
            Err(error("the enrollment ticket response was invalid"))
        }
    }
}

fn revoke_blocking(
    profile: &crate::config::PersistedCloudRelayProfile,
    id: &str,
) -> Result<(), DaemonError> {
    let credential = profile.kernel_credential.as_deref().unwrap_or_default();
    let response = crate::outbound_dial::agent_builder()
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
        _ => Err(error("the unusable enrollment ticket could not be revoked; revoke it in Chariox before retrying")),
    }
}

fn quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./:=-".contains(&b))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

/// One line for the new machine's own shell. The code reaches Setup on stdin through a
/// shell builtin, so it never appears in any process's argument list; the leading space
/// keeps it out of history for shells that ignore space-prefixed lines.
fn installer_command(
    install_url: &str,
    ticket: &str,
    api_url: &str,
) -> Result<String, DaemonError> {
    let install = url::Url::parse(install_url)
        .ok()
        .filter(|u| {
            u.scheme() == "https"
                && u.query().is_none()
                && u.fragment().is_none()
                && u.username().is_empty()
                && u.password().is_none()
        })
        .ok_or_else(|| error("Chariox did not provide a valid installer address"))?;
    if ticket.is_empty()
        || !ticket
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(error("the enrollment ticket has an unexpected form"));
    }
    Ok(format!(
        " (setup_script=$(curl -fsSL {}) && printf '%s\\n' {} | sh -c \"$setup_script\" sh --enroll --api-url {})",
        quote(install.as_str()),
        ticket,
        quote(api_url.trim_end_matches('/'))
    ))
}

fn checked_expiry(expires_at: &str, now: chrono::DateTime<chrono::Utc>) -> Result<(), DaemonError> {
    let expiry = chrono::DateTime::parse_from_rfc3339(expires_at)
        .map_err(|_| error("the enrollment ticket has an invalid expiry"))?;
    let ttl = expiry.signed_duration_since(now);
    if ttl <= chrono::Duration::zero()
        || ttl > chrono::Duration::seconds(TICKET_SECONDS + CLOCK_SKEW_SECONDS)
    {
        return Err(error("the enrollment ticket has an unexpected lifetime"));
    }
    Ok(())
}

fn issue_command(
    profile: &crate::config::PersistedCloudRelayProfile,
) -> Result<OwnMachineEnrollmentCommand, DaemonError> {
    let ticket = issue_blocking(profile)?;
    let built = checked_expiry(&ticket.expires_at, chrono::Utc::now()).and_then(|()| {
        let install_url = ticket
            .setup_install_url
            .as_deref()
            .ok_or_else(|| error("this Chariox service has no machine installer configured"))?;
        installer_command(install_url, &ticket.ticket, &profile.api_url)
    });
    match built {
        Ok(command) => Ok(OwnMachineEnrollmentCommand {
            command,
            expires_at: ticket.expires_at.clone(),
        }),
        Err(refused) => {
            revoke_blocking(profile, &ticket.ticket_id)?;
            Err(refused)
        }
    }
}

pub(crate) async fn execute(
    config: DaemonConfig,
    command: &KernelCommand,
    _request: crate::local::IssueOwnMachineEnrollmentRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let profile = owner_profile(&config, command)?.clone();
    let enrollment = tokio::task::spawn_blocking(move || issue_command(&profile))
        .await
        .map_err(|_| error("the enrollment task failed"))??;
    Ok(LocalDaemonResponse::OwnMachineEnrollment { enrollment })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{IssueOwnMachineEnrollmentRequest, LocalDaemonRequest};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    const TICKET: &str = "Zk3pQ9_aB-tTjHk2mYw8Rx5LdN7vE0cUoF1sGiAhXbM";

    /// A Cloud stand-in on loopback that records "METHOD path body" per request.
    fn cloud(ticket_body: String) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut length = 0usize;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let first = line.trim().to_string();
                log.lock().unwrap().push(format!(
                    "{} {}",
                    first.trim_end_matches(" HTTP/1.1"),
                    String::from_utf8_lossy(&body)
                ));
                let (status, payload) = if first.starts_with("POST") {
                    ("201 Created", ticket_body.clone())
                } else {
                    ("204 No Content", String::new())
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
                    payload.len()
                );
            }
        });
        (url, seen)
    }

    // The files are what Cloud's own ticket route is fed with and what it answers; Cloud checks both against the same files.
    #[test]
    fn the_ticket_request_matches_the_shared_contract_and_its_answer_decodes() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/cloud-contract/own-machine-ticket-request.json"
        ))
        .unwrap();
        assert_eq!(ticket_request_body(), fixture);
        let answer: Ticket = serde_json::from_str(include_str!(
            "../../tests/cloud-contract/own-machine-ticket-response.json"
        ))
        .unwrap();
        assert_eq!(answer.ticket_id, "tk_synthetic");
        assert!(!answer.ticket.is_empty());
        assert!(chrono::DateTime::parse_from_rfc3339(&answer.expires_at).is_ok());
    }

    fn ticket_body(install_url: Option<&str>) -> String {
        let expires = (chrono::Utc::now() + chrono::Duration::seconds(TICKET_SECONDS)).to_rfc3339();
        let mut value = json!({"ticket":TICKET,"ticket_id":"tk_1","expires_at":expires});
        if let Some(url) = install_url {
            value["setup_install_url"] = url.into();
        }
        value.to_string()
    }

    fn signed_in(api_url: &str) -> DaemonConfig {
        let mut config = DaemonConfig::for_tests();
        config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
            api_url: api_url.into(),
            user_id: "owner".into(),
            realm_id: "realm".into(),
            relay_url: "wss://relay.example".into(),
            kernel_credential: Some("kernel-credential".into()),
            ..Default::default()
        });
        config.relay_url = Some("wss://relay.example".into());
        config
    }

    fn command(user: Option<&str>) -> (KernelCommand, IssueOwnMachineEnrollmentRequest) {
        let request = IssueOwnMachineEnrollmentRequest {};
        let mut command = KernelCommand::from_local_request(
            "enroll",
            None,
            None,
            &LocalDaemonRequest::IssueOwnMachineEnrollment(request.clone()),
        );
        command.caller.user_id = user.map(Into::into);
        (command, request)
    }

    #[tokio::test]
    async fn owner_gets_one_line_installer_command_for_an_own_machine() {
        let (api, seen) = cloud(ticket_body(Some("https://downloads.example/install.sh")));
        let (command, request) = command(Some("owner"));
        let LocalDaemonResponse::OwnMachineEnrollment { enrollment } =
            execute(signed_in(&api), &command, request).await.unwrap()
        else {
            panic!("expected an enrollment command")
        };
        assert!(
            !enrollment.command.contains('\n'),
            "one line for the agent's shell"
        );
        assert!(
            enrollment.command.starts_with(" ("),
            "leading space keeps it out of history"
        );
        assert!(enrollment
            .command
            .contains("curl -fsSL https://downloads.example/install.sh"));
        assert!(enrollment
            .command
            .contains(&format!("printf '%s\\n' {TICKET} |")));
        assert!(enrollment.command.contains(&format!("--api-url {api})")));
        let log = seen.lock().unwrap();
        assert_eq!(log.len(), 1, "no ticket is revoked: {log:?}");
        assert!(log[0].starts_with("POST /v1/kernel-enrollment-tickets "));
        assert!(log[0].contains(r#""purpose":"owner_managed_machine""#));
        assert!(
            !log[0].contains("machine_label"),
            "the machine names itself: {}",
            log[0]
        );
    }

    #[tokio::test]
    async fn a_guests_agent_is_refused_before_cloud_is_contacted() {
        let (api, seen) = cloud(ticket_body(Some("https://downloads.example/install.sh")));
        let (command, request) = command(Some("guest"));
        let refusal = execute(signed_in(&api), &command, request)
            .await
            .unwrap_err();
        assert!(
            refusal.to_string().contains("only this kernel's owner"),
            "{refusal}"
        );
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_kernel_that_is_not_signed_in_says_what_to_do() {
        let (command, request) = command(Some("owner"));
        let refusal = execute(DaemonConfig::for_tests(), &command, request)
            .await
            .unwrap_err();
        assert!(refusal.to_string().contains("sign in"), "{refusal}");
    }

    #[tokio::test]
    async fn a_cloud_without_an_installer_revokes_the_ticket_it_issued() {
        let (api, seen) = cloud(ticket_body(None));
        let (command, request) = command(Some("owner"));
        let refusal = execute(signed_in(&api), &command, request)
            .await
            .unwrap_err();
        assert!(
            refusal.to_string().contains("no machine installer"),
            "{refusal}"
        );
        assert!(!refusal.to_string().contains(TICKET));
        let log = seen.lock().unwrap();
        assert_eq!(log.len(), 2, "{log:?}");
        assert!(log[1].starts_with("DELETE /v1/kernel-enrollment-tickets/tk_1 "));
    }

    #[test]
    fn installer_address_must_be_plain_https_and_the_code_a_bare_token() {
        for bad in [
            "http://x.example/install.sh",
            "https://u:p@x.example/i",
            "https://x.example/i?a=b",
            "not a url",
        ] {
            assert!(
                installer_command(bad, TICKET, "https://api.example").is_err(),
                "{bad}"
            );
        }
        assert!(installer_command("https://x.example/i", "a b", "https://api.example").is_err());
        assert!(
            installer_command("https://x.example/i", "$(rm -rf ~)", "https://api.example").is_err()
        );
        let spaced =
            installer_command("https://x.example/a b", TICKET, "https://api.example/").unwrap();
        assert!(
            spaced.contains("curl -fsSL 'https://x.example/a%20b'"),
            "{spaced}"
        );
        assert!(spaced.ends_with("--api-url https://api.example)"));
    }

    #[test]
    fn ticket_lifetime_must_match_the_requested_short_life() {
        let now = chrono::Utc::now();
        let at = |s: i64| (now + chrono::Duration::seconds(s)).to_rfc3339();
        assert!(checked_expiry(&at(600), now).is_ok());
        assert!(checked_expiry(&at(-1), now).is_err());
        assert!(checked_expiry(&at(3600), now).is_err());
    }
}
