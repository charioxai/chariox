use super::{payload, AppCatalog, CatalogError, Result, ToolSpec};
use crate::wire::{Message, Outcome, Sender, MAX_DEADLINE_MS, WIRE_VERSION};
use rusqlite::Transaction;
use serde::Serialize;
use serde_json::{json, Value};

/// Captured by the kernel from the submitting human, provider run or authorized
/// automation. These identifiers are descriptive; they never prove approval.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Actor {
    Human(String),
    Agent(String),
    Background(String),
}

/// No arbitrary metadata, history, prompt, transcript or credential field.
/// The kernel retains richer authenticated provider/binding context privately.
#[derive(Debug, Clone)]
pub struct CallerContext {
    pub actor: Actor,
    pub room_id: String,
    pub operation_id: String,
    pub task_id: Option<String>,
    pub turn_id: Option<String>,
}
impl CallerContext {
    fn value(&self, installation: &str) -> Result<Value> {
        let actor_id = match &self.actor {
            Actor::Human(id) | Actor::Agent(id) | Actor::Background(id) => id,
        };
        for id in [
            Some(actor_id.as_str()),
            Some(self.room_id.as_str()),
            Some(self.operation_id.as_str()),
            self.task_id.as_deref(),
            self.turn_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if id.is_empty()
                || id.len() > 128
                || id
                    .chars()
                    .any(|ch| ch.is_control() || ch.is_whitespace() || ch == '\u{feff}')
            {
                return Err(CatalogError::Invalid);
            }
        }
        let mut context = json!({"installation_id": installation, "room_id": self.room_id,
            "operation_id": self.operation_id, "actor": self.actor});
        if let Actor::Agent(id) = &self.actor {
            context["agent_id"] = json!(id);
        }
        if let Some(id) = &self.task_id {
            context["task_id"] = json!(id);
        }
        if let Some(id) = &self.turn_id {
            context["turn_id"] = json!(id);
        }
        Ok(context)
    }
}

/// Owns one immutable request correlation. The worker actor assigns a unique ID,
/// sends it through the shared peer, and consumes this value once on response.
/// This value has no permission, receipt, cancellation or scheduling authority.
pub struct ValidatedToolCall<'a> {
    catalog: &'a AppCatalog,
    name: String,
    request: Message,
}

impl AppCatalog {
    /// Validate after the kernel's common operation-policy check. The caller
    /// must keep policy/installation admission serialized until enqueue, and
    /// cancel/fence this request on revoke or generation change. No DB lock may
    /// be held while awaiting App code or user validation.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare<'a>(
        &'a self,
        transaction: &Transaction<'_>,
        trusted_owner: &str,
        name: &str,
        input: Value,
        caller: &CallerContext,
        request_id: &str,
        now_ms: u64,
        deadline_ms: u64,
    ) -> Result<ValidatedToolCall<'a>> {
        self.require_current(transaction, trusted_owner)?;
        let tool = self.tools.get(name).ok_or(CatalogError::UnknownTool)?;
        if deadline_ms <= now_ms || deadline_ms - now_ms > MAX_DEADLINE_MS {
            return Err(CatalogError::Deadline);
        }
        payload::validate(&input)?;
        if !tool.input.is_valid(&input) {
            return Err(CatalogError::Input);
        }
        let request = Message::Request {
            version: WIRE_VERSION,
            generation: self.generation().to_string(),
            id: request_id.into(),
            method: "tools.invoke".into(),
            params: json!({"name": tool.spec.local_name, "input": input}),
            deadline_ms,
            context: Some(caller.value(self.installation_id())?),
        };
        request
            .validate(&self.generation().to_string(), Sender::Supervisor)
            .map_err(|_| CatalogError::Invalid)?;
        Ok(ValidatedToolCall {
            catalog: self,
            name: name.into(),
            request,
        })
    }
}

impl ValidatedToolCall<'_> {
    pub fn request(&self) -> &Message {
        &self.request
    }
    pub fn tool(&self) -> &ToolSpec {
        &self.catalog.tools[&self.name].spec
    }

    /// A stale/late result is discarded, not evidence that its external effect
    /// was undone. The worker actor owns cancellation, broker revocation fences,
    /// terminal correlation and process termination for an uncooperative App.
    pub fn accept(
        self,
        transaction: &Transaction<'_>,
        trusted_owner: &str,
        response: Message,
        now_ms: u64,
    ) -> Result<Value> {
        self.catalog.require_current(transaction, trusted_owner)?;
        let Message::Request {
            id: expected_id,
            deadline_ms,
            generation,
            ..
        } = &self.request
        else {
            unreachable!()
        };
        if now_ms >= *deadline_ms {
            return Err(CatalogError::Deadline);
        }
        response
            .validate(generation, Sender::Worker)
            .map_err(|_| CatalogError::Response)?;
        let Message::Response { id, outcome, .. } = response else {
            return Err(CatalogError::Response);
        };
        if id != *expected_id {
            return Err(CatalogError::Response);
        }
        match outcome {
            Outcome::Failure(failure) => Err(CatalogError::Worker(failure.error)),
            Outcome::Success(success) => {
                payload::validate(&success.result)?;
                if self.catalog.tools[&self.name]
                    .output
                    .as_ref()
                    .is_some_and(|schema| !schema.is_valid(&success.result))
                {
                    return Err(CatalogError::Output);
                }
                Ok(success.result)
            }
        }
    }
}
