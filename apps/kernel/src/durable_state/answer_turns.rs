//! A delivery consumed inside another provider turn points to that turn's durable receipt.
use super::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::provider::AnsweredTurnReceipt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const ANSWER_TURN_EVENT: &str = "session.prompt.answer_turn";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnswerTurnRecord {
    pub turn: AnsweredTurnReceipt,
    pub consumed_calls: BTreeSet<String>,
}

impl DurableKernelStateStore {
    pub(crate) fn record_answer_turn(
        &self,
        task_id: &str,
        answer: &AnsweredTurnReceipt,
        consumed_call: &str,
    ) -> Result<(), DaemonError> {
        let previous = self.answer_turn(task_id)?;
        let mut record = AnswerTurnRecord {
            turn: answer.clone(),
            consumed_calls: previous
                .as_ref()
                .map(|record| record.consumed_calls.clone())
                .unwrap_or_default(),
        };
        record.consumed_calls.insert(consumed_call.to_string());
        if previous.as_ref() == Some(&record) {
            return Ok(());
        }
        self.append_event(
            ANSWER_TURN_EVENT,
            Some(task_id.to_string()),
            serde_json::json!(record),
        )?;
        Ok(())
    }

    pub(crate) fn answer_turn(
        &self,
        task_id: &str,
    ) -> Result<Option<AnswerTurnRecord>, DaemonError> {
        self.load_subject_events_by_kind(task_id, ANSWER_TURN_EVENT, 1)?
            .pop()
            .map(|event| {
                serde_json::from_value(event.payload).map_err(|error| DaemonError::LocalTransport {
                    operation: "durable_state.answer_turn",
                    message: format!("The answering turn receipt is invalid: {error}"),
                })
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_turn_receipt_survives_reopen_and_repeated_delivery() {
        let directory = crate::test_support::TestTempDir::new("answer-turn-receipt");
        let path = directory.join("state.db");
        let first = AnsweredTurnReceipt {
            session_id: "room".into(),
            agent_id: "child".into(),
            prompt_id: "q1".into(),
            task_id: "task-q1".into(),
        };
        let last = AnsweredTurnReceipt {
            prompt_id: "q2".into(),
            task_id: "task-q2".into(),
            ..first.clone()
        };
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            store
                .record_answer_turn("held-h", &first, "background-1")
                .unwrap();
            store
                .record_answer_turn("held-h", &last, "background-2")
                .unwrap();
            store
                .record_answer_turn("held-h", &last, "background-2")
                .unwrap();
            assert_eq!(
                store
                    .load_subject_events_by_kind("held-h", ANSWER_TURN_EVENT, 10)
                    .unwrap()
                    .len(),
                2
            );
        }
        let reopened = DurableKernelStateStore::open(path).unwrap();
        let receipt = reopened.answer_turn("held-h").unwrap().unwrap();
        assert_eq!(receipt.turn, last);
        assert_eq!(
            receipt.consumed_calls,
            BTreeSet::from(["background-1".into(), "background-2".into()])
        );
        assert!(reopened.answer_turn("unrelated").unwrap().is_none());
    }
}
