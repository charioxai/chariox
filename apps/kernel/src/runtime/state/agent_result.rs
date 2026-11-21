//! A child publishes a file from its private scratch. Its parent gets an
//! opaque handle, then materializes the verified bytes into its own scratch.
use super::*;
use crate::artifacts::{ArtifactRecord, OperationalArtifactStore, StoreArtifactRequest};
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};

mod files;

const TOOL: &str = "chariox.agents.result";
const KIND: &str = "agent_result";
const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(serde::Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum ResultAction {
    Publish { filename: String },
    Fetch { artifact_id: String },
}

pub(super) fn is_result_tool(name: &str) -> bool {
    let name = name.strip_prefix("mcp__chariox__").unwrap_or(name);
    matches!(name, TOOL | "chariox_agents_result")
}

pub(super) fn spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: TOOL.into(),
        description: "Return a large file to your direct parent on this kernel. Save it in your own TMPDIR and publish it by filename (one file, at most 16 MiB). Include the artifact_id in your final answer. As the parent, fetch the artifact_id to get a local path in your own TMPDIR, plus size and SHA-256. The contents stay out of the reply.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["publish", "fetch"]},
                "filename": {"type": "string", "description": "With publish: the single filename in your own TMPDIR."},
                "artifact_id": {"type": "string", "description": "With fetch: the handle a child returned in its final answer."}
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    }
}

impl KernelRuntimeState {
    pub(super) fn agent_result_available(&self, run: &crate::provider::RuntimeProviderRun) -> bool {
        self.room_agent_tools_enabled()
            && run.agent_instance_id().is_some()
            && run.state() != crate::provider::ProviderRunState::Ended
            && !self
                .owned
                .provider_run_projection
                .is_leased_provider_run(run.id())
    }

    pub(super) async fn agent_result_tool(
        &self,
        token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let run = self
            .provider_run_for_auth_token(token)
            .filter(|run| self.agent_result_available(run))
            .ok_or_else(|| {
                refused("Run this operation from an active Room agent on the home kernel")
            })?;
        let agent_id = run.agent_instance_id().expect("Room agent was checked");
        let scratch = crate::agent_temp::dir(run.session_id(), agent_id)
            .ok_or_else(|| refused("This agent has no private scratch directory"))?;
        let action: ResultAction = serde_json::from_value(arguments)
            .map_err(|error| refused(format!("Invalid result arguments: {error}")))?;
        let config = self.owned.config_projection.snapshot();
        let scope = match &action {
            ResultAction::Publish { .. } => {
                let child = self.owned.agent_store.get_agent(agent_id)?;
                let parent = child
                    .parent_agent_id()
                    .and_then(|id| self.owned.agent_store.get_agent(id).ok())
                    .filter(|parent| !parent.archived() && parent.remote_execution().is_none())
                    .ok_or_else(|| refused("Publishing requires a direct parent on this kernel"))?;
                parent.id().to_owned()
            }
            ResultAction::Fetch { .. } => agent_id.to_owned(),
        };
        let payload = tokio::task::spawn_blocking(move || {
            let store = OperationalArtifactStore::open(
                config.operational_artifact_root(),
                config.operational_artifact_index_path(),
            )?;
            match action {
                ResultAction::Publish { filename } => publish(&store, &scratch, filename, scope),
                ResultAction::Fetch { artifact_id } => {
                    fetch(&store, &scratch, &artifact_id, &scope)
                }
            }
        })
        .await
        .map_err(|error| refused(format!("Result operation stopped: {error}")))??;
        Ok(RuntimeToolResult { ok: true, payload })
    }
}

fn publish(
    store: &OperationalArtifactStore,
    scratch: &Path,
    filename: String,
    parent_id: String,
) -> Result<serde_json::Value, DaemonError> {
    let source = files::open_source(scratch, &filename).map_err(refused)?;
    let artifact = store.store_open_file(
        StoreArtifactRequest {
            source_path: source,
            display_name: filename,
            source_kind: KIND.into(),
            media_type: None,
            enqueue_archive: false,
            session_id: None,
            attachment_id: None,
            workspace_id: None,
            worktree_path: None,
            metadata: [("recipient_agent_id".into(), parent_id.into())].into(),
        },
        MAX_BYTES as u64,
    )?;
    Ok(receipt(&artifact))
}

fn fetch(
    store: &OperationalArtifactStore,
    scratch: &Path,
    artifact_id: &str,
    agent_id: &str,
) -> Result<serde_json::Value, DaemonError> {
    let unavailable =
        || refused("Result unavailable. Use the artifact ID from your child's final answer");
    let artifact = store
        .load_artifact(artifact_id)?
        .filter(|record| {
            record.source_kind == KIND
                && record
                    .metadata
                    .get("recipient_agent_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(agent_id)
        })
        .ok_or_else(unavailable)?;
    let data = store.read_artifact_chunk(&artifact, 0, MAX_BYTES)?;
    if !data.eof || crate::workflow_code::sha256_hex(&data.data) != artifact.sha256 {
        return Err(refused(
            "The saved result failed verification. Ask the child to publish it again",
        ));
    }
    let name = format!("result-{}", artifact.artifact_id);
    files::materialize(scratch, &name, &data.data).map_err(refused)?;
    let mut result = receipt(&artifact);
    result["path"] = serde_json::json!(scratch.join(name));
    Ok(result)
}

fn receipt(artifact: &ArtifactRecord) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": artifact.artifact_id,
        "filename": artifact.display_name,
        "size_bytes": artifact.size_bytes,
        "sha256": artifact.sha256,
    })
}

fn refused(message: impl Into<String>) -> DaemonError {
    DaemonError::LocalTransport {
        operation: TOOL,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestTempDir;

    #[test]
    fn agent_result_survives_removing_the_source_and_reopening_the_store() {
        let root = TestTempDir::new("agent-result");
        let child = root.join("child");
        let parent = root.join("parent");
        std::fs::create_dir(&child).unwrap();
        std::fs::create_dir(&parent).unwrap();
        let input = format!(r#"{{"data":"{}"}}"#, "あ".repeat(68_263));
        assert_eq!(input.len(), 204_800);
        let file = child.join("result.json");
        std::fs::write(&file, &input).unwrap();
        let open =
            || OperationalArtifactStore::open(root.join("store"), root.join("index")).unwrap();
        let published = publish(&open(), &child, "result.json".into(), "parent".into()).unwrap();
        let id = published["artifact_id"].as_str().unwrap();
        assert_eq!(published["size_bytes"], input.len());
        assert_eq!(
            published["sha256"],
            crate::workflow_code::sha256_hex(input.as_bytes())
        );
        std::fs::remove_file(file).unwrap();
        let reopened = open();
        assert!(fetch(&reopened, &parent, id, "peer").is_err());
        let fetched = fetch(&reopened, &parent, id, "parent").unwrap();
        let path = Path::new(fetched["path"].as_str().unwrap());
        assert_eq!(path.parent(), Some(parent.as_path()));
        assert_eq!(std::fs::read(path).unwrap(), input.as_bytes());
        assert_eq!(fetch(&reopened, &parent, id, "parent").unwrap(), fetched);
        std::fs::write(path, "changed").unwrap();
        assert!(fetch(&reopened, &parent, id, "parent").is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"changed");
    }

    #[test]
    fn agent_result_source_stays_in_the_childs_scratch() {
        let root = TestTempDir::new("agent-result-scope");
        let child = root.join("child");
        let outside = root.join("outside");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(&outside, "outside").unwrap();
        std::os::unix::fs::symlink(&outside, child.join("link")).unwrap();
        std::fs::hard_link(&outside, child.join("hardlink")).unwrap();
        for name in ["../outside", outside.to_str().unwrap(), "link", "hardlink"] {
            assert!(files::open_source(&child, name).is_err(), "{name}");
        }
    }
}
