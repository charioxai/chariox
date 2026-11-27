//! Row 678 (owner decision, 2026-11-26): the kernel's own watchdog for a wedge it cannot heal.
//!
//! Trigger, and nothing else: the app lock held continuously by ONE holder for longer than the
//! lifetime of the relay token the kernel holds (its `exp - iat`, the token's own number). Many
//! short holders never add up (each new holder starts a new age), and a Cloud or network failure
//! never holds the app lock, so neither triggers it.
//!
//! Then: one log line naming the holder, one try of the product's hot reload, and only if that
//! cannot proceed an exit, and only when something will start the kernel again (a systemd unit
//! with `Restart=on-failure`/`always`, or a supervisor that declares itself). Unsupervised, the
//! kernel stays up and reports itself degraded. After a supervised restart the kernel tells its
//! owner that it restarted itself, and why.
//!
//! It runs on its own thread: the runtime it watches may be what stopped advancing.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::runtime::app_lock::AppLockHolder;
use crate::runtime::router::CommandRouter;

const MARKER_FILE: &str = "kernel-self-restart.json";
/// A supervisor that starts the kernel again after a failed exit and is not a systemd unit (the
/// macOS launchd job, the managed bootstrap supervisor) names itself here.
const SUPERVISOR_ENV: &str = "CHARIOX_KERNEL_SUPERVISOR";

/// What to do about one long hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    Healthy,
    /// One holder has held the app lock past the token lifetime.
    Wedged {
        holder: AppLockHolder,
        token_lifetime: Duration,
    },
}

/// The relay token's own lifetime, `exp - iat`; none without a readable token.
pub(crate) fn token_lifetime(token: Option<&str>) -> Option<Duration> {
    let payload = crate::runtime::cloud_relay_control::relay_token_payload(token?)?;
    let issued = payload.get("iat")?.as_u64()?;
    let expires = payload.get("exp")?.as_u64()?;
    let lifetime = expires.checked_sub(issued).filter(|seconds| *seconds > 0)?;
    Some(Duration::from_secs(lifetime))
}

pub(crate) fn judge(holder: Option<AppLockHolder>, token_lifetime: Option<Duration>) -> Verdict {
    match (holder, token_lifetime) {
        (Some(holder), Some(token_lifetime)) if holder.held > token_lifetime => Verdict::Wedged {
            holder,
            token_lifetime,
        },
        _ => Verdict::Healthy,
    }
}

/// Who starts this kernel again if it exits with a failure; `None` means nobody would.
pub(crate) fn restart_supervisor() -> Option<String> {
    if let Some(named) = std::env::var(SUPERVISOR_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        return Some(named);
    }
    systemd_restart_supervisor()
}

#[cfg(target_os = "linux")]
fn systemd_restart_supervisor() -> Option<String> {
    std::env::var_os("INVOCATION_ID")?;
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let (unit, user) = systemd_unit_of(&cgroup)?;
    let mut command = std::process::Command::new("systemctl");
    if user {
        command.arg("--user");
    }
    command
        .args(["show", "--property=Restart", "--value", &unit])
        .env_clear();
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    for (key, value) in systemctl_environment(user, uid, |key| std::env::var_os(key)) {
        command.env(key, value);
    }
    let output = match command.output() {
        Ok(output) if output.status.success() => output,
        failed => {
            crate::logging::warn_with_fields(
                "kernel.wedge_watchdog",
                "the systemd unit's restart policy could not be read; the kernel treats itself as unsupervised",
                serde_json::json!({
                    "unit": unit,
                    "user_unit": user,
                    "error": match &failed {
                        Ok(output) => String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                        Err(error) => error.to_string(),
                    },
                }),
            );
            return None;
        }
    };
    let policy = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    restarts_after_failed_exit(&policy).then(|| format!("systemd unit {unit} (Restart={policy})"))
}

/// What `systemctl` needs to reach its manager: a user unit's manager is found through the user
/// bus (`XDG_RUNTIME_DIR`, or `/run/user/<uid>` when it is unset, and `DBUS_SESSION_BUS_ADDRESS`).
pub(crate) fn systemctl_environment(
    user: bool,
    uid: u32,
    get: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Vec<(&'static str, std::ffi::OsString)> {
    let mut environment = vec![("PATH", std::ffi::OsString::from("/usr/bin:/bin"))];
    if user {
        environment.push((
            "XDG_RUNTIME_DIR",
            get("XDG_RUNTIME_DIR").unwrap_or_else(|| format!("/run/user/{uid}").into()),
        ));
        if let Some(bus) = get("DBUS_SESSION_BUS_ADDRESS") {
            environment.push(("DBUS_SESSION_BUS_ADDRESS", bus));
        }
    }
    environment
}

#[cfg(not(target_os = "linux"))]
fn systemd_restart_supervisor() -> Option<String> {
    None
}

/// The service unit in this process's cgroup path, and whether it is a user unit.
pub(crate) fn systemd_unit_of(cgroup: &str) -> Option<(String, bool)> {
    let path = cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?
        .trim();
    let unit = path
        .split('/')
        .rev()
        .find(|segment| segment.ends_with(".service"))?;
    let user = path.contains("/user@");
    Some((unit.to_owned(), user))
}

/// The systemd policies that start a unit again after its main process exits with a failure.
pub(crate) fn restarts_after_failed_exit(policy: &str) -> bool {
    matches!(policy, "on-failure" | "always")
}

pub(crate) fn start(router: Arc<CommandRouter>) {
    let runtime = tokio::runtime::Handle::try_current().ok();
    let spawned = std::thread::Builder::new()
        .name("chariox-wedge-watch".into())
        .spawn(move || watch(router, runtime));
    if let Err(error) = spawned {
        crate::logging::warn_with_fields(
            "kernel.wedge_watchdog",
            "the kernel's wedge watchdog could not start",
            serde_json::json!({ "error": error.to_string() }),
        );
    }
}

fn watch(router: Arc<CommandRouter>, runtime: Option<tokio::runtime::Handle>) {
    let tick = crate::runtime::stall_watchdog::STALL_THRESHOLD;
    let mut degraded = false;
    loop {
        std::thread::sleep(tick);
        let config = router.relay_config_snapshot();
        let verdict = judge(
            crate::runtime::app_lock::current_holder(),
            token_lifetime(config.relay_token.as_deref()),
        );
        match verdict {
            Verdict::Healthy => {
                if degraded {
                    degraded = false;
                    crate::logging::info_with_fields(
                        "kernel.wedge_watchdog",
                        "the operation that held the kernel finished; the kernel is no longer degraded",
                        serde_json::json!({}),
                    );
                }
            }
            Verdict::Wedged { .. } if degraded => {}
            Verdict::Wedged {
                holder,
                token_lifetime,
            } => {
                let supervisor = restart_supervisor();
                crate::logging::error_with_fields(
                    "kernel.wedge_watchdog",
                    "one operation has held the kernel for longer than its relay token lasts",
                    serde_json::json!({
                        "holder_operation": holder.operation,
                        "holder_thread": holder.thread,
                        "held_ms": holder.held.as_millis(),
                        "token_lifetime_ms": token_lifetime.as_millis(),
                        "restart_supervisor": supervisor,
                    }),
                );
                let reason = Reason {
                    holder_operation: holder.operation.to_owned(),
                    held_ms: u64::try_from(holder.held.as_millis()).unwrap_or(u64::MAX),
                    token_lifetime_ms: u64::try_from(token_lifetime.as_millis())
                        .unwrap_or(u64::MAX),
                };
                let marker = config.private_runtime_state_root().join(MARKER_FILE);
                match supervisor {
                    Some(supervisor) => recover(&supervisor, &reason, &marker),
                    None => {
                        degraded = true;
                        crate::logging::error_with_fields(
                            "kernel.wedge_watchdog",
                            "the kernel is degraded and stays up: nothing would start it again if it exited",
                            serde_json::json!({ "holder_operation": holder.operation }),
                        );
                        notify_owner_detached(
                            Arc::clone(&router),
                            runtime.clone(),
                            DEGRADED_TITLE,
                            degraded_body(&reason),
                        );
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Reason {
    holder_operation: String,
    held_ms: u64,
    token_lifetime_ms: u64,
}

const RESTARTED_TITLE: &str = "This kernel restarted itself";
const DEGRADED_TITLE: &str = "This kernel is held up";

fn minutes(ms: u64) -> u64 {
    ms.div_ceil(60_000).max(1)
}

fn restarted_body(reason: &Reason) -> String {
    format!(
        "One of its operations did not finish for over {} min and it could not recover in place, so it restarted. Agents that were working show Interrupted: press Resume to continue.",
        minutes(reason.held_ms)
    )
}

fn degraded_body(reason: &Reason) -> String {
    format!(
        "One of its operations has not finished for over {} min. Work that needs it waits. Restart the kernel when you can.",
        minutes(reason.held_ms)
    )
}

/// Supervised: try the product's hot reload once, then exit so the supervisor starts the kernel.
/// The reason is saved first: a reload that succeeds is a restart the owner is told about too.
fn recover(supervisor: &str, reason: &Reason, marker: &Path) {
    write_marker_bounded(marker, reason);
    crate::logging::warn_with_fields(
        "kernel.wedge_watchdog",
        "trying a hot reload of the kernel",
        serde_json::json!({ "restart_supervisor": supervisor }),
    );
    match crate::hot_reload::request_restart_from_thread() {
        Ok(()) => {
            crate::logging::warn_with_fields(
                "kernel.wedge_watchdog",
                "hot reload prepared; waiting for it to replace the kernel",
                serde_json::json!({}),
            );
            std::thread::sleep(crate::hot_reload::reload_patience());
            crate::logging::error_with_fields(
                "kernel.wedge_watchdog",
                "the prepared hot reload did not replace the kernel",
                serde_json::json!({}),
            );
        }
        Err(refusal) => crate::logging::error_with_fields(
            "kernel.wedge_watchdog",
            "hot reload could not proceed",
            serde_json::json!({ "reason": refusal }),
        ),
    }
    crate::logging::error_with_fields(
        "kernel.wedge_watchdog",
        "the kernel exits so its supervisor starts it again",
        serde_json::json!({ "restart_supervisor": supervisor }),
    );
    std::process::exit(1);
}

/// The marker tells the next start why. A stalled disk may be the wedge itself, so the write gets
/// the stall threshold and no more: the restart matters more than its explanation.
fn write_marker_bounded(marker: &Path, reason: &Reason) {
    let (done, finished) = std::sync::mpsc::channel();
    let path = marker.to_path_buf();
    let body = serde_json::to_vec(reason).unwrap_or_default();
    let _ = std::thread::Builder::new()
        .name("chariox-wedge-marker".into())
        .spawn(move || {
            let _ = done.send(std::fs::write(path, body));
        });
    match finished.recv_timeout(crate::runtime::stall_watchdog::STALL_THRESHOLD) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => crate::logging::warn_with_fields(
            "kernel.wedge_watchdog",
            "the restart reason could not be saved",
            serde_json::json!({ "error": error.to_string() }),
        ),
        Err(_) => crate::logging::warn_with_fields(
            "kernel.wedge_watchdog",
            "the restart reason could not be saved in time",
            serde_json::json!({}),
        ),
    }
}

/// At start: a marker left by a self-restart becomes a notice for the owner.
pub(crate) fn report_self_restart(router: &CommandRouter) {
    let config = router.relay_config_snapshot();
    let marker = config.private_runtime_state_root().join(MARKER_FILE);
    let Some(reason) = take_marker(&marker) else {
        return;
    };
    crate::logging::warn_with_fields(
        "kernel.wedge_watchdog",
        "the kernel restarted itself after an operation held it",
        serde_json::json!({
            "holder_operation": reason.holder_operation,
            "held_ms": reason.held_ms,
            "token_lifetime_ms": reason.token_lifetime_ms,
        }),
    );
    record_owner_notice(router, RESTARTED_TITLE, restarted_body(&reason));
}

fn take_marker(marker: &PathBuf) -> Option<Reason> {
    let body = std::fs::read(marker).ok()?;
    let _ = std::fs::remove_file(marker);
    serde_json::from_slice(&body).ok()
}

fn notify_owner_detached(
    router: Arc<CommandRouter>,
    runtime: Option<tokio::runtime::Handle>,
    title: &'static str,
    body: String,
) {
    // Its own thread: the notice is durable and the disk may be what is stalled.
    let _ = std::thread::Builder::new()
        .name("chariox-wedge-notice".into())
        .spawn(move || {
            let _entered = runtime.as_ref().map(tokio::runtime::Handle::enter);
            record_owner_notice(&router, title, body);
        });
}

fn record_owner_notice(router: &CommandRouter, title: &str, body: String) {
    // The kernel's owner holds its accounts under the local identity: the Inbox shows a kernel
    // notice to whoever acts for that identity, and to no guest.
    let owner = crate::session::DEFAULT_LOCAL_USER_ID.to_owned();
    let runtime_state = router.runtime_state();
    let created_at_ms = crate::session::unix_epoch_ms();
    let stored = runtime_state.durable_state_store().record_person_notice(
        crate::durable_state::StoredNotice {
            card_id: format!("notice_{created_at_ms}_{:016x}", rand::random::<u64>()),
            owner_user_id: owner,
            session_id: crate::durable_state::WORKFLOW_RELINK_ORIGIN.to_owned(),
            agent_id: crate::durable_state::WORKFLOW_RELINK_ORIGIN.to_owned(),
            title: title.to_owned(),
            body: Some(body),
            link: None,
            priority: crate::local::PersonInboxPriority::High,
            created_at_ms,
            read: false,
            folded: 0,
        },
    );
    match stored {
        Ok(_) => runtime_state.record_person_inbox_change(),
        Err(error) => crate::logging::warn_with_fields(
            "kernel.wedge_watchdog",
            "the owner notice could not be saved",
            serde_json::json!({ "error": error.to_string() }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn token(claims: serde_json::Value) -> String {
        format!(
            "header.{}.signature",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&claims).unwrap())
        )
    }

    fn holder(held_ms: u64) -> AppLockHolder {
        AppLockHolder {
            operation: "renewal save",
            thread: "tokio-rt-worker".into(),
            held: Duration::from_millis(held_ms),
        }
    }

    #[test]
    fn the_token_lifetime_is_its_own_exp_minus_iat() {
        let lifetime = token_lifetime(Some(&token(serde_json::json!({"iat": 100, "exp": 400}))));
        assert_eq!(lifetime, Some(Duration::from_secs(300)));
        assert_eq!(token_lifetime(None), None);
        assert_eq!(
            token_lifetime(Some(&token(serde_json::json!({"exp": 400})))),
            None
        );
    }

    /// Row 678: only one hold longer than the token's own lifetime is a wedge; without a token
    /// the kernel has no number and never acts.
    #[test]
    fn only_one_hold_past_the_token_lifetime_is_a_wedge() {
        let lifetime = Some(Duration::from_secs(300));
        assert_eq!(judge(Some(holder(299_000)), lifetime), Verdict::Healthy);
        assert!(matches!(
            judge(Some(holder(301_000)), lifetime),
            Verdict::Wedged { .. }
        ));
        assert_eq!(judge(None, lifetime), Verdict::Healthy);
        assert_eq!(judge(Some(holder(10_000_000)), None), Verdict::Healthy);
    }

    #[test]
    fn a_systemd_unit_is_read_from_the_cgroup_and_only_restarting_policies_count() {
        assert_eq!(
            systemd_unit_of("0::/system.slice/lanes-executor-kernel.service\n"),
            Some(("lanes-executor-kernel.service".into(), false))
        );
        assert_eq!(
            systemd_unit_of(
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/chariox-kernel.service/supervisor\n"
            ),
            Some(("chariox-kernel.service".into(), true))
        );
        assert_eq!(systemd_unit_of("0::/user.slice/session-3.scope\n"), None);
        assert!(restarts_after_failed_exit("on-failure"));
        assert!(restarts_after_failed_exit("always"));
        assert!(!restarts_after_failed_exit("no"));
        assert!(!restarts_after_failed_exit("on-abnormal"));
    }

    /// Review P1 (b325d8c): `systemctl --user` reaches the user manager only through the user bus.
    #[test]
    fn a_user_unit_lookup_keeps_the_user_bus() {
        let from = |pairs: &'static [(&'static str, &'static str)]| {
            move |key: &str| {
                pairs
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| std::ffi::OsString::from(value))
            }
        };
        let kept = systemctl_environment(
            true,
            1000,
            from(&[
                ("XDG_RUNTIME_DIR", "/run/user/1000"),
                ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
                ("CHARIOX_HOME", "/home/u/.chariox"),
            ]),
        );
        assert_eq!(
            kept,
            vec![
                ("PATH", "/usr/bin:/bin".into()),
                ("XDG_RUNTIME_DIR", "/run/user/1000".into()),
                (
                    "DBUS_SESSION_BUS_ADDRESS",
                    "unix:path=/run/user/1000/bus".into()
                ),
            ]
        );
        let derived = systemctl_environment(true, 1000, from(&[]));
        assert_eq!(derived[1], ("XDG_RUNTIME_DIR", "/run/user/1000".into()));
        assert_eq!(
            systemctl_environment(false, 0, from(&[("XDG_RUNTIME_DIR", "/run/user/0")])),
            vec![("PATH", "/usr/bin:/bin".into())]
        );
    }

    #[test]
    fn the_owner_is_told_in_product_words() {
        let reason = Reason {
            holder_operation: "x::y::{{closure}}".into(),
            held_ms: 310_000,
            token_lifetime_ms: 300_000,
        };
        let body = restarted_body(&reason);
        assert!(body.contains("over 6 min"));
        assert!(!body.contains("closure"));
        assert!(degraded_body(&reason).contains("Restart the kernel"));
    }
}
