//! The `publish` step of a Room's setup: the Room's own agent creates the repository with the
//! machine's own CLI login and pushes. Chariox holds no host-specific publishing logic.
use super::*;
use crate::provider::{ProviderRunState, ProviderUtilityExecutionPolicy};
use crate::runtime::agent_utility_executor::{
    run_provider_utility_prompt, AgentUtilityPromptParts,
};
use crate::session::RoomPublishRequest;

const OPERATION: &str = "publish room";
/// Creating a repository and pushing is a few commands, with a login that may need a moment.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(300);
const PROVIDER_READY_TIMEOUT: Duration = Duration::from_secs(60);
const REASON_MAX_CHARS: usize = 160;
const HIDDEN_CONTEXT: &str = "You are setting up a new Room for the person. Run the commands needed in the current folder, then answer with exactly one line as the user message asks and nothing else.";

pub(super) fn publish_prompt(request: &RoomPublishRequest, folder_name: &str) -> String {
    let visibility = if request.private { "private" } else { "public" };
    let name = request
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(folder_name);
    format!(
        "Create a {visibility} GitHub repository named `{name}` for this folder using this machine's own CLI login (`gh`, or whatever login exists here). \
Add it as the Git remote `origin` and push the current branch. Do not change any files other than Git's own configuration. \
Your whole answer must be one single line: `PUBLISHED <repository url>` when the push succeeded, or `FAILED <short reason that says what to do next>` when it did not."
    )
}

/// The step's result from the agent's answer: its last non-empty line is the verdict.
pub(super) fn parse_publish_answer(answer: &str) -> Result<(), String> {
    let verdict = answer
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or_default();
    if let Some(url) = verdict.strip_prefix("PUBLISHED") {
        if !url.trim().is_empty() {
            return Ok(());
        }
    }
    if let Some(reason) = verdict.strip_prefix("FAILED") {
        let reason = reason.trim();
        if !reason.is_empty() {
            return Err(short_reason(reason));
        }
    }
    Err("the agent did not say whether publishing worked; check the repository, then retry".into())
}

pub(super) fn short_reason(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    if line.chars().count() > REASON_MAX_CHARS {
        let cut: String = line.chars().take(REASON_MAX_CHARS - 1).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

impl KernelRuntimeState {
    /// Runs the publish step on the Room's first agent and reports it as the step's result.
    pub(super) async fn run_room_publish(
        &self,
        session_id: &str,
        agent_id: &str,
        request: &RoomPublishRequest,
    ) -> Result<(), String> {
        let folder_name = self
            .owned
            .session_store
            .get_session(session_id)
            .ok()
            .and_then(|session| {
                std::path::Path::new(session.workspace_id())
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "workspace".to_string());
        let prompt = publish_prompt(request, &folder_name);
        match self.room_publish_answer(session_id, agent_id, prompt).await {
            Ok(answer) => parse_publish_answer(&answer),
            Err(error) => {
                tracing::warn!(session_id, %error, "room publish turn failed");
                Err("the agent could not run the publish turn; check its login, then retry".into())
            }
        }
    }

    async fn room_publish_answer(
        &self,
        session_id: &str,
        agent_id: &str,
        visible_user_prompt: String,
    ) -> Result<String, DaemonError> {
        let started = tokio::time::Instant::now();
        let run = loop {
            let (_, run) = self
                .agent_utility_provider_run(session_id, agent_id, OPERATION)
                .await?;
            match run.state() {
                ProviderRunState::Running => break run,
                ProviderRunState::Starting if started.elapsed() < PROVIDER_READY_TIMEOUT => {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                state => {
                    return Err(DaemonError::LocalTransport {
                        operation: OPERATION,
                        message: format!("the agent's provider is not running ({state:?})"),
                    })
                }
            }
        };
        // The narrowest existing policy that lets the agent run commands in the Room's folder.
        run_provider_utility_prompt(
            self,
            run,
            AgentUtilityPromptParts {
                visible_user_prompt,
                hidden_system_context: HIDDEN_CONTEXT.into(),
            },
            OPERATION,
            ProviderUtilityExecutionPolicy::ExistingRun,
            PUBLISH_TIMEOUT,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_line_is_the_verdict() {
        assert_eq!(
            parse_publish_answer("working\nPUBLISHED https://github.com/me/x\n"),
            Ok(())
        );
        assert_eq!(
            parse_publish_answer("PUBLISHED https://github.com/me/x\nthen more text"),
            Err(
                "the agent did not say whether publishing worked; check the repository, then retry"
                    .into()
            )
        );
    }

    #[test]
    fn a_failure_keeps_one_short_line() {
        let long = format!("FAILED {}", "x".repeat(400));
        let reason = parse_publish_answer(&long).unwrap_err();
        assert_eq!(reason.chars().count(), REASON_MAX_CHARS);
        assert_eq!(
            parse_publish_answer("FAILED not logged in; run `gh auth login`"),
            Err("not logged in; run `gh auth login`".into())
        );
    }

    #[test]
    fn an_empty_or_bare_verdict_fails() {
        assert!(parse_publish_answer("").is_err());
        assert!(parse_publish_answer("PUBLISHED").is_err());
        assert!(parse_publish_answer("FAILED").is_err());
    }

    #[test]
    fn the_prompt_names_visibility_and_the_default_name() {
        let private = RoomPublishRequest {
            private: true,
            name: None,
        };
        let prompt = publish_prompt(&private, "my-folder");
        assert!(prompt.contains("private GitHub repository named `my-folder`"));
        let public = RoomPublishRequest {
            private: false,
            name: Some(" chosen ".into()),
        };
        assert!(publish_prompt(&public, "my-folder")
            .contains("public GitHub repository named `chosen`"));
    }
}
