//! `chariox.vault.capture_command`: a command prints a secret (an API key a
//! CLI just created, `openssl rand` output); the kernel stores it in the Vault
//! and the agent receives only the handle and its own label.
use super::elevation::VaultElevationKind;
use super::*;
use crate::config::{UserCredentialConfig, UserCredentialMetadataConfig};
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub(crate) const VAULT_CAPTURE_COMMAND: &str = "chariox.vault.capture_command";
pub(super) const CAPTURED_KIND: &str = "vault_capture";
const VALUE_LIMIT: usize = 16 * 1024;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureArgs {
    request_id: String,
    argv: serde_json::Value,
    #[serde(default)]
    cwd: Option<String>,
    host: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    extract: Extract,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

#[derive(serde::Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Extract {
    #[default]
    Stdout,
    Json {
        pointer: String,
    },
    Regex {
        pattern: String,
        #[serde(default = "first_group")]
        group: usize,
    },
}

fn first_group() -> usize {
    1
}

pub(crate) fn capture_command_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: VAULT_CAPTURE_COMMAND.into(),
        description: "During your sudo window, run one command (argv, no shell) in your workspace and store a secret it prints on stdout in the owner's Vault. You never see the value or the command's output: the result is an opaque credential_id with your label. extract picks the value: {\"kind\":\"stdout\"} (whole stdout, trimmed; the default), {\"kind\":\"json\",\"pointer\":\"/data/api_key\"} (JSON Pointer into stdout), or {\"kind\":\"regex\",\"pattern\":\"token: (\\\\S+)\",\"group\":1}. host names the service the secret belongs to. Use it with chariox.vault.use_env. Retrying with the same request_id returns the same credential without running the command again.".into(),
        input_schema: serde_json::json!({"type":"object","required":["request_id","argv","host"],"properties":{
            "request_id":{"type":"string","minLength":1,"maxLength":128},
            "argv":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":64},
            "cwd":{"type":"string","description":"Directory inside your workspace, relative to it"},
            "host":{"type":"string","description":"Host of the service the secret is for, such as api.example.com"},
            "label":{"type":"string","maxLength":200},
            "extract":{"type":"object","required":["kind"],"properties":{
                "kind":{"type":"string","enum":["stdout","json","regex"]},
                "pointer":{"type":"string"},
                "pattern":{"type":"string","maxLength":1024},
                "group":{"type":"integer","minimum":0}},"additionalProperties":false},
            "timeout_seconds":{"type":"integer","minimum":1,"maximum":MAX_TIMEOUT_SECONDS,"default":DEFAULT_TIMEOUT_SECONDS}},
            "additionalProperties":false}),
    }
}

impl KernelRuntimeState {
    pub(crate) async fn vault_capture_command(
        &self,
        token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let elevation = self.vault_elevation_allows(token, VaultElevationKind::Capture)?;
        let args: CaptureArgs = serde_json::from_value(arguments)
            .map_err(|e| error(format!("invalid Vault capture arguments: {e}")))?;
        if args.request_id.trim().is_empty() || args.request_id.len() > 128 {
            return Err(error("request_id must be 1-128 characters"));
        }
        if args.label.as_ref().is_some_and(|label| label.len() > 200) {
            return Err(error("label must be at most 200 characters"));
        }
        let host = service_host(&args.host)?;
        let argv = super::super::agent_process_watch::command_argv(&args.argv)?;
        let pattern = match &args.extract {
            Extract::Regex { pattern, group } => {
                let regex = regex::RegexBuilder::new(pattern)
                    .size_limit(1 << 20)
                    .build()
                    .map_err(|e| error(format!("invalid extract pattern: {e}")))?;
                if *group >= regex.captures_len() {
                    return Err(error("extract group is not in the pattern"));
                }
                Some(regex)
            }
            _ => None,
        };
        let timeout = command_timeout(args.timeout_seconds)?;
        let turn = &elevation.turn;
        let handle = captured_handle(turn, &args.request_id);
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        if let Some(existing) = registry.get(&handle)? {
            return committed(&existing, turn, &host);
        }
        let cwd =
            super::super::agent_process_watch::workspace_cwd(&elevation.run, args.cwd.as_deref())?;
        // Unlock before running: a key a service shows once must not be lost.
        let _unlock = self
            .ensure_vault_unlocked_for_agent(&turn.session_id, &turn.agent_id, "vault_capture")
            .await?;
        self.recheck_vault_elevation(token, &elevation)?;
        let output = run_command(
            self,
            elevation.run.clone(),
            super::super::agent_process_watch::KernelCommand {
                argv,
                cwd,
                env: Vec::new(),
                stdin: None,
                timeout,
            },
        )
        .await?;
        use super::super::agent_process_watch::KernelCommandExit;
        match output.exit {
            KernelCommandExit::Exited(0) => {}
            KernelCommandExit::Exited(code) => {
                return Err(error(format!(
                    "the command exited with code {code}; nothing was stored{}",
                    stderr_excerpt(&output.stderr)
                )))
            }
            KernelCommandExit::TimedOut => {
                return Err(error(format!(
                    "the command did not finish within {} s and was stopped; nothing was stored",
                    timeout.as_secs()
                )))
            }
            KernelCommandExit::Lost => {
                return Err(error(
                    "the command could not be settled; nothing was stored",
                ))
            }
        }
        if output.truncated {
            return Err(error(
                "the command printed more than 1 MiB; nothing was stored",
            ));
        }
        let secret = extract(&output.stdout, &args.extract, pattern.as_ref())?;
        let now = crate::session::unix_epoch_ms();
        let credential = UserCredentialConfig {
            id: handle.clone(),
            description: args.label.clone(),
            source: crate::config::UserCredentialSourceConfig::Vault {
                key: handle.clone(),
            },
            allowed_hosts: vec![host.clone()],
            allowed_uses: vec![crate::config::UserCredentialUse::Env],
            injection: crate::config::UserCredentialInjectionConfig::Env,
            metadata: Some(UserCredentialMetadataConfig {
                created_by_kind: Some(CAPTURED_KIND.into()),
                created_by_id: Some(turn.agent_id.clone()),
                session_id: Some(turn.session_id.clone()),
                provider: None,
                provider_run_id: turn.provider_run_id.clone(),
                vault_key: Some(handle.clone()),
                created_at_ms: Some(now),
                updated_at_ms: Some(now),
            }),
        };
        // Retain the submitting turn across lifecycle/barrier/app waits, as generation does.
        let authority = LocalDaemonRequest::ListSessions(crate::local::ListSessionsRequest);
        let bound = self
            .with_external_command_authority(Some((&turn.entry_id, &authority)))
            .with_sudo_binding(turn.prompt_id.clone().zip(turn.provider_run_id.clone()));
        bound
            .upsert_observed_vault_credential(
                &self.home_runtime_secret_service()?,
                &registry,
                credential,
                secret.as_str(),
                false,
            )
            .await?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"credential_id": handle, "label": args.label,
                "host": host, "length": secret.chars().count(), "created": true}),
        })
    }
}

/// A retry returns the committed handle only to the same agent, session and host.
fn committed(
    existing: &UserCredentialConfig,
    turn: &KernelSudoTurn,
    host: &str,
) -> Result<RuntimeToolResult, DaemonError> {
    let metadata = existing.metadata.as_ref();
    if metadata.and_then(|m| m.created_by_kind.as_deref()) != Some(CAPTURED_KIND)
        || metadata.and_then(|m| m.created_by_id.as_deref()) != Some(turn.agent_id.as_str())
        || metadata.and_then(|m| m.session_id.as_deref()) != Some(turn.session_id.as_str())
        || existing.allowed_hosts != [host]
    {
        return Err(error(
            "request_id already names a different credential; use a new request_id",
        ));
    }
    Ok(RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({"credential_id": existing.id, "label": existing.description,
            "host": host, "created": false}),
    })
}

/// Deterministic per request so a lost acknowledgement never stores a second value.
fn captured_handle(turn: &KernelSudoTurn, request_id: &str) -> String {
    let digest = Sha256::digest(
        [
            CAPTURED_KIND,
            turn.owner_user_id.as_str(),
            &turn.session_id,
            &turn.agent_id,
            request_id.trim(),
        ]
        .join("\n"),
    );
    format!("cap-{}", &format!("{digest:x}")[..24])
}

/// A bare host name (with an optional port), as credential hosts are matched.
fn service_host(raw: &str) -> Result<String, DaemonError> {
    let raw = raw.trim();
    let url = url::Url::parse(&format!("https://{raw}"))
        .ok()
        .filter(|url| url.path() == "/" && url.query().is_none() && url.username().is_empty())
        .filter(|_| !raw.contains(['/', '?', '#', '@']));
    match url
        .as_ref()
        .and_then(|url| url.host_str().map(|h| (h, url.port())))
    {
        Some((host, Some(port))) => Ok(format!("{host}:{port}")),
        Some((host, None)) => Ok(host.to_string()),
        None => Err(error("host must be a host name such as api.example.com")),
    }
}

/// Errors name what went wrong, never the output they looked at.
fn extract(
    stdout: &[u8],
    how: &Extract,
    pattern: Option<&regex::Regex>,
) -> Result<Zeroizing<String>, DaemonError> {
    let text = std::str::from_utf8(stdout)
        .map_err(|_| error("stdout is not UTF-8 text; nothing was stored"))?;
    let value = match how {
        Extract::Stdout => Zeroizing::new(text.trim().to_string()),
        Extract::Json { pointer } => {
            let json: serde_json::Value = serde_json::from_str(text)
                .map_err(|_| error("stdout is not JSON; nothing was stored"))?;
            match json.pointer(pointer) {
                Some(serde_json::Value::String(value)) => Zeroizing::new(value.trim().to_string()),
                Some(serde_json::Value::Number(value)) => Zeroizing::new(value.to_string()),
                Some(_) => return Err(error(
                    "the JSON pointer names an object, array, boolean or null; nothing was stored",
                )),
                None => {
                    return Err(error(
                        "the JSON pointer matched nothing; nothing was stored",
                    ))
                }
            }
        }
        Extract::Regex { group, .. } => {
            let captures = pattern
                .and_then(|p| p.captures(text))
                .ok_or_else(|| error("the pattern matched nothing; nothing was stored"))?;
            Zeroizing::new(
                captures
                    .get(*group)
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_default(),
            )
        }
    };
    if value.is_empty() {
        return Err(error("the extracted value is empty; nothing was stored"));
    }
    if value.len() > VALUE_LIMIT {
        return Err(error(
            "the extracted value is too large; nothing was stored",
        ));
    }
    Ok(value)
}

/// The tail of a failed command's stderr, sanitized; stdout is never shown.
fn stderr_excerpt(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines = text
        .lines()
        .map(|line| super::super::agent_process_output::sanitize(line.as_bytes()))
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let tail = lines[lines.len().saturating_sub(5)..].join("\n");
    match tail.char_indices().rev().nth(511) {
        _ if tail.is_empty() => String::new(),
        Some((start, _)) => format!(": …{}", &tail[start..]),
        None => format!(": {tail}"),
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn capture_extracts_without_echoing_and_hosts_are_bare() {
        let regex = regex::Regex::new(r"token: (\S+)").unwrap();
        let rx = Extract::Regex {
            pattern: String::new(),
            group: 1,
        };
        assert_eq!(
            extract(b"created\ntoken: sk_123\n", &rx, Some(&regex))
                .unwrap()
                .as_str(),
            "sk_123"
        );
        let json = Extract::Json {
            pointer: "/data/key".into(),
        };
        assert_eq!(
            extract(br#"{"data":{"key":" k-1 "}}"#, &json, None)
                .unwrap()
                .as_str(),
            "k-1"
        );
        assert_eq!(
            extract(b"  abc\n", &Extract::Stdout, None)
                .unwrap()
                .as_str(),
            "abc"
        );
        for (stdout, how) in [
            (&b"secret-in-output"[..], &json),
            (&b"{\"data\":{}}"[..], &json),
            (&b"   \n"[..], &Extract::Stdout),
        ] {
            let message = extract(stdout, how, None).unwrap_err().to_string();
            assert!(!message.contains("secret-in-output"), "{message}");
        }
        assert_eq!(service_host("api.example.com").unwrap(), "api.example.com");
        assert_eq!(service_host("127.0.0.1:8123").unwrap(), "127.0.0.1:8123");
        for refused in ["", "https://x.com", "x.com/path", "u@x.com", "a b"] {
            assert!(service_host(refused).is_err(), "{refused}");
        }
    }
}
