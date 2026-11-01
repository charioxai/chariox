//! MP-08/MP-10/MP-11: loopback gateways have the kernel's lifetime, never a relay tunnel.
use super::agent_trigger_http::http_error;
use super::*;
use crate::session::AgentTrigger;
use std::collections::BTreeMap;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

#[derive(Clone, Default)]
pub(super) struct AgentTriggerGatewayStore(Arc<tokio::sync::Mutex<GatewayState>>);
#[derive(Default)]
struct GatewayState {
    children: BTreeMap<String, Child>,
    retry_at: BTreeMap<String, u64>,
}

impl KernelRuntimeState {
    pub(super) async fn start_agent_trigger_gateway(
        &self,
        session_id: &str,
        trigger: &AgentTrigger,
    ) -> Result<(), DaemonError> {
        let crate::session::AgentTriggerSource::Http { ingress_id, port } = trigger.source() else {
            return Ok(());
        };
        let key = format!("{session_id}:{}", trigger.id());
        let mut gateways = self.owned.agent_trigger_gateways.0.lock().await;
        if let Some(child) = gateways.children.get_mut(&key) {
            if child
                .try_wait()
                .map_err(|e| http_error(e.to_string()))?
                .is_none()
            {
                return Ok(());
            }
        }
        gateways.children.remove(&key);
        let result = launch_gateway(self, session_id, trigger.id(), ingress_id, port.get()).await;
        match result {
            Ok(child) => {
                gateways.children.insert(key.clone(), child);
                gateways.retry_at.remove(&key);
                Ok(())
            }
            Err(error) => {
                gateways
                    .retry_at
                    .insert(key, crate::session::unix_epoch_ms().saturating_add(5000));
                Err(error)
            }
        }
    }

    pub(super) async fn reconcile_agent_trigger_gateways(&self) {
        let desired: BTreeMap<_, _> = self
            .owned
            .session_store
            .list_non_ended_sessions_including_hidden()
            .into_iter()
            .flat_map(|session| {
                session
                    .agent_triggers()
                    .iter()
                    .filter(|t| {
                        matches!(t.source(), crate::session::AgentTriggerSource::Http { .. })
                    })
                    .map(|t| {
                        (
                            format!("{}:{}", session.id(), t.id()),
                            (session.id().to_owned(), t.clone()),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        {
            let mut gateways = self.owned.agent_trigger_gateways.0.lock().await;
            gateways.children.retain(|key, child| {
                if desired.contains_key(key) {
                    return true;
                }
                if child.id().is_some_and(|pid| pid > 1) {
                    let _ = child.start_kill();
                }
                false
            });
            gateways.retry_at.retain(|key, _| desired.contains_key(key));
        }
        for (key, (session_id, trigger)) in desired {
            let retry_at = self
                .owned
                .agent_trigger_gateways
                .0
                .lock()
                .await
                .retry_at
                .get(&key)
                .copied()
                .unwrap_or(0);
            if retry_at > crate::session::unix_epoch_ms() {
                continue;
            }
            if let Err(error) = self
                .start_agent_trigger_gateway(&session_id, &trigger)
                .await
            {
                let _ = self.owned.session_store.write().mark_agent_trigger_failed(
                    &session_id,
                    trigger.id(),
                    crate::session::unix_epoch_ms(),
                    error.to_string(),
                );
                let _ = self
                    .owned
                    .persist_agent_trigger_session(&session_id, "agent_trigger_gateway_failed");
            }
        }
    }
}

async fn launch_gateway(
    state: &KernelRuntimeState,
    session: &str,
    trigger: &str,
    ingress: &str,
    port: u16,
) -> Result<Child, DaemonError> {
    let cli = super::workflow_publication_runtime_lifecycle::resolve_chariox_cli_bin()?;
    // A cold checkout may need a build. Finish it before the HTTP readiness
    // budget starts, rather than killing its launcher while pnpm is still running.
    let prepared = Command::new(&cli)
        .arg("serve")
        .arg("prepare")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .map_err(|e| http_error(format!("cannot prepare local HTTP gateway: {e}")))?;
    if !prepared.success() {
        return Err(http_error(format!(
            "local HTTP gateway build failed ({prepared}); run chariox-cli serve prepare and retry"
        )));
    }
    let mut command = Command::new(cli);
    command
        .arg("serve")
        .arg("agent-trigger")
        .arg(session)
        .arg(trigger)
        .arg(port.to_string())
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--kernel-url")
        .arg(
            state
                .owned
                .config_projection
                .snapshot()
                .kernel_websocket_url(),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .env("CHARIOX_PUBLICATION_EXIT_ON_STDIN_CLOSE", "1");
    let mut child = command
        .spawn()
        .map_err(|e| http_error(format!("cannot start local HTTP gateway: {e}")))?;
    if child.id().is_none_or(|pid| pid <= 1) {
        return Err(http_error(
            "gateway did not return a valid child process id",
        ));
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().map_err(|e| http_error(e.to_string()))? {
            return Err(http_error(format!("local HTTP gateway exited ({status}); check the server build and recreate the trigger")));
        }
        if gateway_ready(port, ingress).await {
            return Ok(child);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(http_error(
                "local HTTP gateway did not become ready; check the server build and loopback port",
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn gateway_ready(port: u16, ingress: &str) -> bool {
    tokio::time::timeout(std::time::Duration::from_millis(250), async {
        let mut stream = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST,port)).await.ok()?;
        stream.write_all(format!("GET /{ingress}/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes()).await.ok()?;
        let mut bytes = Vec::new(); stream.take(4096).read_to_end(&mut bytes).await.ok()?;
        let body = String::from_utf8(bytes).ok()?;
        Some(body.starts_with("HTTP/1.1 200") && body.contains(ingress))
    }).await.ok().flatten().unwrap_or(false)
}
