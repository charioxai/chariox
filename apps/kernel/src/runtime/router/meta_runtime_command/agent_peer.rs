//! Room commands that talk to another agent: `agent message` and `agent steer`. Both are the
//! `chariox.send_agent_message` tool, so the authority rule, the sender stamp and the
//! cross-kernel relay are that tool's.
use super::result::{meta_command_error, meta_command_failure_result};
use super::*;

/// How a room command names an agent outside the caller's room: `--room`, and the kernel
/// (`--kernel` or `--machine`, the same references `agent spawn` takes) only with a room.
#[derive(Default)]
pub(super) struct PeerTarget<'a> {
    pub(super) room: Option<&'a str>,
    pub(super) kernel: Option<&'a str>,
    pub(super) reply_requested: bool,
}

impl<'a> PeerTarget<'a> {
    /// `None` is a usage error.
    pub(super) fn parse(options: &'a [String], reply_flag: bool) -> Option<Self> {
        let mut target = Self::default();
        let mut options = options.iter();
        while let Some(flag) = options.next() {
            match flag.as_str() {
                "--room" if target.room.is_none() => {
                    target.room = Some(options.next()?.as_str());
                }
                "--kernel" | "--machine" if target.kernel.is_none() => {
                    target.kernel = Some(options.next()?.as_str());
                }
                "--reply-requested" if reply_flag && !target.reply_requested => {
                    target.reply_requested = true;
                }
                _ => return None,
            }
        }
        (target.kernel.is_none() || target.room.is_some()).then_some(target)
    }
}

impl CommandRouter {
    /// `agent message <agent> <text> [--room R [--kernel K|--machine M]] [--reply-requested]` and
    /// `agent steer <agent> <text> [--room R [--kernel K|--machine M]] [--reply-requested]`.
    pub(super) async fn dispatch_meta_agent_message_command(
        &self,
        provider_run: Option<&crate::provider::RuntimeProviderRun>,
        session: &crate::session::RuntimeSession,
        sender: &crate::agent::AgentInstance,
        command: &str,
        steer: bool,
        args: &[String],
    ) -> RuntimeToolResult {
        let usage = || {
            meta_command_failure_result(
                command,
                meta_command_error(if steer {
                    "usage: agent steer <agent-ref> <text> [--room <room> [--kernel <kernel>|--machine <machine>]] [--reply-requested]; the text is one quoted argument"
                } else {
                    "usage: agent message <agent-ref> <text> [--room <room> [--kernel <kernel>|--machine <machine>]] [--reply-requested]; the text is one quoted argument"
                }),
            )
        };
        let [agent, text, options @ ..] = args else {
            return usage();
        };
        let Some(target) = PeerTarget::parse(options, true) else {
            return usage();
        };
        let (Some(provider_run), Some(prompt)) =
            (provider_run, session.active_prompt_for_agent(sender.id()))
        else {
            return meta_command_failure_result(
                command,
                meta_command_error("a message is sent from a running turn of the caller"),
            );
        };
        // Another kernel's answer can be lost: the same call again in this turn is then the
        // same request, so it lands once and never reaches a later turn twice.
        let idempotency_key = target.kernel.map(|kernel| {
            use sha2::{Digest, Sha256};
            let call = serde_json::json!([
                sender.id(),
                prompt.id(),
                kernel,
                target.room,
                agent,
                text,
                steer,
                target.reply_requested,
            ]);
            Sha256::digest(call.to_string())[..16]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        });
        let message = serde_json::to_value(crate::transport::runtime_tools::SendAgentMessageArgs {
            agent: agent.clone(),
            room: target.room.map(str::to_owned),
            kernel: target.kernel.map(str::to_owned),
            message: text.clone(),
            origin_prompt_id: prompt.id().to_owned(),
            urgent: steer,
            reply_requested: target.reply_requested,
            attachments: Vec::new(),
            idempotency_key,
        });
        let result = match message {
            Ok(arguments) => {
                self.runtime_state
                    .send_agent_message_for_provider_run(provider_run, session, sender, arguments)
                    .await
            }
            Err(error) => Err(meta_command_error(error.to_string())),
        };
        match result {
            Ok(sent) if sent.ok => RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({"command": command, "response": sent.payload}),
            },
            Ok(refused) => meta_command_failure_result(command, refused.payload),
            Err(error) => meta_command_failure_result(command, error),
        }
    }

    /// `agent promote <message-id>`: the sender makes a message it queued urgent.
    pub(super) async fn dispatch_meta_agent_promote_command(
        &self,
        provider_run: Option<&crate::provider::RuntimeProviderRun>,
        session: &crate::session::RuntimeSession,
        sender: &crate::agent::AgentInstance,
        command: &str,
        args: &[String],
    ) -> RuntimeToolResult {
        let [message_id] = args else {
            return meta_command_failure_result(
                command,
                meta_command_error("usage: agent promote <message-id>"),
            );
        };
        let Some(provider_run) = provider_run else {
            return meta_command_failure_result(
                command,
                meta_command_error("a message is promoted from a running turn of the sender"),
            );
        };
        let result = self
            .runtime_state
            .promote_agent_message_for_provider_run(
                provider_run,
                session,
                sender,
                serde_json::json!({ "message_id": message_id }),
            )
            .await;
        match result {
            Ok(promoted) if promoted.ok => RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({"command": command, "response": promoted.payload}),
            },
            Ok(refused) => meta_command_failure_result(command, refused.payload),
            Err(error) => meta_command_failure_result(command, error),
        }
    }
}
