//! The person's Inbox state on this kernel (docs/NOTIFICATION_CENTER_PLAN.md sections 4.1 and
//! 4.4): the for-your-information cards agents send, and the read, snooze and mute choices, so
//! they follow the person to every client. Needs-you cards are the Rooms' interactions and are
//! not stored here.
use super::DurableKernelStateStore;
use crate::error::DaemonError;
use crate::local::{InboxCardChange, PersonInboxOrigin, PersonInboxPriority};
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const NOTICE: &str = "person_inbox.notice";
const WORKFLOW_START_REFUSED: &str = "person_inbox.workflow_start_refused";
const WORKFLOW_STARTED: &str = "person_inbox.workflow_started";
pub(crate) const WORKFLOW_START_NOTICE_PREFIX: &str = "workflow-start:";
pub(crate) const WORKFLOW_RELINK_ORIGIN: &str = "kernel";
const WORKFLOW_START_NOTICE_PATTERN: &str = "workflow-start:*";
const CARD_STATE: &str = "person_inbox.card_state";
const MUTE: &str = "person_inbox.mute";
/// The kernel's existing retention for finished records.
const RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
/// Per owner; past it the oldest read notice goes first, then the oldest.
const NOTICES_PER_OWNER: i64 = 1_024;
/// An origin may send this many notices in `NOTICE_WINDOW_MS`; the further ones fold into one
/// card flagged Noisy (plan 5.1, 7.6).
pub(crate) const NOTICE_RATE_LIMIT: usize = 5;
pub(crate) const NOTICE_WINDOW_MS: u64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredNotice {
    pub(crate) card_id: String,
    pub(crate) owner_user_id: String,
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) title: String,
    pub(crate) body: Option<String>,
    pub(crate) link: Option<String>,
    pub(crate) priority: PersonInboxPriority,
    pub(crate) created_at_ms: u64,
    pub(crate) read: bool,
    /// 0 for one notice; n when this card stands for n notices sent past the rate limit, the
    /// latest one in `title`/`body`/`link`.
    pub(crate) folded: u32,
}

impl StoredNotice {
    pub(crate) fn is_workflow_start_refusal(&self) -> bool {
        self.card_id.starts_with(WORKFLOW_START_NOTICE_PREFIX)
    }

    pub(crate) fn is_workflow_relink(&self) -> bool {
        self.is_workflow_start_refusal() && self.agent_id == WORKFLOW_RELINK_ORIGIN
    }
}

#[derive(Serialize, Deserialize)]
struct CardState {
    owner_user_id: String,
    card_ids: Vec<String>,
    change: InboxCardChange,
    at_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct Mute {
    owner_user_id: String,
    origin: PersonInboxOrigin,
    muted: bool,
}

/// One owner's stored Inbox state, as of `now_ms`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PersonInboxState {
    /// Newest first.
    pub(crate) notices: Vec<StoredNotice>,
    pub(crate) snoozes: BTreeMap<String, u64>,
    pub(crate) mutes: Vec<PersonInboxOrigin>,
}

pub(super) fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS person_inbox_notices (
            card_id TEXT PRIMARY KEY, owner_user_id TEXT NOT NULL, session_id TEXT NOT NULL,
            agent_id TEXT NOT NULL, title TEXT NOT NULL, body TEXT, link TEXT,
            priority TEXT NOT NULL, created_at_ms INTEGER NOT NULL, read INTEGER NOT NULL,
            folded INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS person_inbox_notices_owner
            ON person_inbox_notices (owner_user_id, created_at_ms);
        CREATE TABLE IF NOT EXISTS person_inbox_snoozes (
            owner_user_id TEXT NOT NULL, card_id TEXT NOT NULL, until_ms INTEGER NOT NULL,
            PRIMARY KEY (owner_user_id, card_id)
        );
        CREATE TABLE IF NOT EXISTS person_inbox_mutes (
            owner_user_id TEXT NOT NULL, session_id TEXT NOT NULL, agent_id TEXT NOT NULL,
            PRIMARY KEY (owner_user_id, session_id, agent_id)
        );
        CREATE TRIGGER IF NOT EXISTS person_inbox_session_deleted
        AFTER INSERT ON durable_state_events WHEN NEW.kind = 'session.deleted'
        BEGIN
            DELETE FROM person_inbox_notices WHERE session_id = NEW.subject_id;
            DELETE FROM person_inbox_mutes WHERE session_id = NEW.subject_id;
        END;",
    )
}

pub(super) fn apply_event(tx: &Transaction<'_>, kind: &str, payload: &str) -> rusqlite::Result<()> {
    let decode =
        |error: serde_json::Error| rusqlite::Error::ToSqlConversionFailure(Box::new(error));
    match kind {
        NOTICE | WORKFLOW_START_REFUSED => {
            let notice: StoredNotice = serde_json::from_str(payload).map_err(decode)?;
            let repeat = if kind == WORKFLOW_START_REFUSED {
                1
            } else {
                notice.folded
            };
            if repeat > 0 {
                let updated = tx.execute(
                    "UPDATE person_inbox_notices SET folded = min(folded + ?2, 4294967295), title = ?3, body = ?4,
                        link = ?5, created_at_ms = ?6, read = ?7,
                        priority = CASE WHEN priority = 'high' OR ?8 = 'high' THEN 'high' ELSE 'normal' END
                    WHERE card_id = ?1",
                    params![
                        notice.card_id,
                        repeat,
                        notice.title,
                        notice.body,
                        notice.link,
                        notice.created_at_ms as i64,
                        notice.read,
                        priority_text(notice.priority)
                    ],
                )?;
                if updated != 0 {
                    return prune(tx, &notice.owner_user_id, notice.created_at_ms);
                }
            }
            tx.execute(
                "INSERT OR IGNORE INTO person_inbox_notices (card_id, owner_user_id, session_id,
                    agent_id, title, body, link, priority, created_at_ms, read, folded)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    notice.card_id,
                    notice.owner_user_id,
                    notice.session_id,
                    notice.agent_id,
                    notice.title,
                    notice.body,
                    notice.link,
                    priority_text(notice.priority),
                    notice.created_at_ms as i64,
                    notice.read,
                    notice.folded
                ],
            )?;
            prune(tx, &notice.owner_user_id, notice.created_at_ms)
        }
        WORKFLOW_STARTED => {
            let pattern: String = serde_json::from_str(payload).map_err(decode)?;
            tx.execute(
                "DELETE FROM person_inbox_snoozes WHERE card_id GLOB ?1",
                [&pattern],
            )?;
            tx.execute(
                "DELETE FROM person_inbox_notices WHERE card_id GLOB ?1",
                [&pattern],
            )?;
            Ok(())
        }
        CARD_STATE => {
            let state: CardState = serde_json::from_str(payload).map_err(decode)?;
            for card_id in &state.card_ids {
                match state.change {
                    InboxCardChange::Read { read } => tx.execute(
                        "UPDATE person_inbox_notices SET read = ?3
                        WHERE owner_user_id = ?1 AND card_id = ?2",
                        params![state.owner_user_id, card_id, read],
                    )?,
                    InboxCardChange::Snooze { until_ms: Some(until) } => tx.execute(
                        "INSERT INTO person_inbox_snoozes (owner_user_id, card_id, until_ms)
                        VALUES (?1, ?2, ?3)
                        ON CONFLICT(owner_user_id, card_id) DO UPDATE SET until_ms = excluded.until_ms",
                        params![state.owner_user_id, card_id, until as i64],
                    )?,
                    InboxCardChange::Snooze { until_ms: None } => tx.execute(
                        "DELETE FROM person_inbox_snoozes WHERE owner_user_id = ?1 AND card_id = ?2",
                        params![state.owner_user_id, card_id],
                    )?,
                };
            }
            prune(tx, &state.owner_user_id, state.at_ms)
        }
        MUTE => {
            let mute: Mute = serde_json::from_str(payload).map_err(decode)?;
            let sql = if mute.muted {
                "INSERT OR IGNORE INTO person_inbox_mutes (owner_user_id, session_id, agent_id)
                VALUES (?1, ?2, ?3)"
            } else {
                "DELETE FROM person_inbox_mutes
                WHERE owner_user_id = ?1 AND session_id = ?2 AND agent_id = ?3"
            };
            tx.execute(
                sql,
                params![
                    mute.owner_user_id,
                    mute.origin.session_id,
                    mute.origin.agent_id
                ],
            )?;
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Drops expired and excess notices. A refused workflow start stays active until another
/// start succeeds or its Room is removed.
fn prune(tx: &Transaction<'_>, owner_user_id: &str, now_ms: u64) -> rusqlite::Result<()> {
    tx.execute(
        "DELETE FROM person_inbox_notices
         WHERE owner_user_id = ?1 AND created_at_ms < ?2 AND card_id NOT GLOB ?3",
        params![
            owner_user_id,
            now_ms.saturating_sub(RETENTION_MS) as i64,
            WORKFLOW_START_NOTICE_PATTERN
        ],
    )?;
    tx.execute(
        "DELETE FROM person_inbox_snoozes WHERE owner_user_id = ?1 AND until_ms <= ?2",
        params![owner_user_id, now_ms as i64],
    )?;
    tx.execute(
        "DELETE FROM person_inbox_notices WHERE card_id IN (
            SELECT card_id FROM person_inbox_notices WHERE owner_user_id = ?1 AND card_id NOT GLOB ?3
            ORDER BY read DESC, created_at_ms ASC, card_id ASC
            LIMIT max(0, (SELECT count(*) FROM person_inbox_notices WHERE owner_user_id = ?1 AND card_id NOT GLOB ?3) - ?2)
        )",
        params![owner_user_id, NOTICES_PER_OWNER, WORKFLOW_START_NOTICE_PATTERN],
    )?;
    Ok(())
}

fn priority_text(priority: PersonInboxPriority) -> &'static str {
    match priority {
        PersonInboxPriority::Normal => "normal",
        PersonInboxPriority::High => "high",
    }
}

impl DurableKernelStateStore {
    /// Stores a notice; a muted origin's notice arrives read. Past the origin's rate limit the
    /// notice folds into the origin's Noisy card, and the returned notice carries that card's id.
    pub(crate) fn record_person_notice(
        &self,
        mut notice: StoredNotice,
    ) -> Result<StoredNotice, DaemonError> {
        let origin = PersonInboxOrigin {
            session_id: notice.session_id.clone(),
            agent_id: notice.agent_id.clone(),
        };
        let state = self.person_inbox_state(&notice.owner_user_id, notice.created_at_ms)?;
        notice.read = state.mutes.contains(&origin);
        let window_start = notice.created_at_ms.saturating_sub(NOTICE_WINDOW_MS);
        let mut recent = state.notices.iter().filter(|stored| {
            !stored.is_workflow_start_refusal()
                && stored.session_id == notice.session_id
                && stored.agent_id == notice.agent_id
                && stored.created_at_ms > window_start
        });
        if recent.clone().filter(|stored| stored.folded == 0).count() >= NOTICE_RATE_LIMIT {
            if let Some(noisy) = recent.find(|stored| stored.folded > 0) {
                notice.card_id = noisy.card_id.clone();
            }
            notice.folded = 1;
        }
        self.append_event(NOTICE, Some(notice.card_id.clone()), to_payload(&notice)?)?;
        Ok(notice)
    }

    pub(crate) fn record_workflow_start_refusal(
        &self,
        notice: StoredNotice,
    ) -> Result<(), DaemonError> {
        self.append_event(
            WORKFLOW_START_REFUSED,
            Some(notice.card_id.clone()),
            to_payload(&notice)?,
        )?;
        Ok(())
    }

    pub(crate) fn clear_workflow_start_refusals(&self, prefix: &str) -> Result<bool, DaemonError> {
        let pattern = format!("{prefix}*");
        let present: bool = self
            .lock_connection("person_inbox.workflow_start")?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM person_inbox_notices WHERE card_id GLOB ?1)",
                [&pattern],
                |row| row.get(0),
            )
            .map_err(|error| error_for(error.to_string()))?;
        if present {
            self.append_event(WORKFLOW_STARTED, None, to_payload(&pattern)?)?;
        }
        Ok(present)
    }

    pub(crate) fn set_person_inbox_card_state(
        &self,
        owner_user_id: &str,
        card_ids: Vec<String>,
        change: InboxCardChange,
        at_ms: u64,
    ) -> Result<(), DaemonError> {
        let state = CardState {
            owner_user_id: owner_user_id.into(),
            card_ids,
            change,
            at_ms,
        };
        self.append_event(CARD_STATE, None, to_payload(&state)?)?;
        Ok(())
    }

    pub(crate) fn set_person_inbox_mute(
        &self,
        owner_user_id: &str,
        origin: PersonInboxOrigin,
        muted: bool,
    ) -> Result<(), DaemonError> {
        let mute = Mute {
            owner_user_id: owner_user_id.into(),
            origin,
            muted,
        };
        self.append_event(MUTE, None, to_payload(&mute)?)?;
        Ok(())
    }

    /// Notices within retention, newest first, and the snoozes that have not ended.
    pub(crate) fn person_inbox_state(
        &self,
        owner_user_id: &str,
        now_ms: u64,
    ) -> Result<PersonInboxState, DaemonError> {
        let connection = self.lock_connection("person_inbox.read")?;
        let read = || -> rusqlite::Result<PersonInboxState> {
            let mut notices = connection.prepare(
                "SELECT card_id, session_id, agent_id, title, body, link, priority, created_at_ms, read,
                    folded
                FROM person_inbox_notices WHERE owner_user_id = ?1 AND (created_at_ms >= ?2 OR card_id GLOB ?3)
                ORDER BY created_at_ms DESC, card_id ASC",
            )?;
            let notices = notices
                .query_map(
                    params![
                        owner_user_id,
                        now_ms.saturating_sub(RETENTION_MS) as i64,
                        WORKFLOW_START_NOTICE_PATTERN
                    ],
                    |row| {
                        Ok(StoredNotice {
                            card_id: row.get(0)?,
                            owner_user_id: owner_user_id.into(),
                            session_id: row.get(1)?,
                            agent_id: row.get(2)?,
                            title: row.get(3)?,
                            body: row.get(4)?,
                            link: row.get(5)?,
                            priority: if row.get::<_, String>(6)? == "high" {
                                PersonInboxPriority::High
                            } else {
                                PersonInboxPriority::Normal
                            },
                            created_at_ms: row.get::<_, i64>(7)? as u64,
                            read: row.get(8)?,
                            folded: row.get(9)?,
                        })
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut snoozes = connection.prepare(
                "SELECT card_id, until_ms FROM person_inbox_snoozes
                WHERE owner_user_id = ?1 AND until_ms > ?2",
            )?;
            let snoozes = snoozes
                .query_map(params![owner_user_id, now_ms as i64], |row| {
                    Ok((row.get(0)?, row.get::<_, i64>(1)? as u64))
                })?
                .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
            let mut mutes = connection.prepare(
                "SELECT session_id, agent_id FROM person_inbox_mutes WHERE owner_user_id = ?1
                ORDER BY session_id, agent_id",
            )?;
            let mutes = mutes
                .query_map(params![owner_user_id], |row| {
                    Ok(PersonInboxOrigin {
                        session_id: row.get(0)?,
                        agent_id: row.get(1)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(PersonInboxState {
                notices,
                snoozes,
                mutes,
            })
        };
        read().map_err(|error| error_for(error.to_string()))
    }
}

fn to_payload(value: &impl Serialize) -> Result<serde_json::Value, DaemonError> {
    serde_json::to_value(value).map_err(|error| error_for(error.to_string()))
}

fn error_for(message: String) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "person_inbox",
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(card_id: &str, created_at_ms: u64) -> StoredNotice {
        StoredNotice {
            card_id: card_id.into(),
            owner_user_id: "owner".into(),
            session_id: "room-1".into(),
            agent_id: "agent-1".into(),
            title: format!("Done {card_id}"),
            body: None,
            link: None,
            priority: PersonInboxPriority::Normal,
            created_at_ms,
            read: false,
            folded: 0,
        }
    }

    #[test]
    fn workflow_refusals_fold_by_caller_and_clear_only_the_accepted_endpoint() {
        let dir = crate::test_support::TestWorktree::new("workflow-start-notice");
        let path = dir.path().join("state.sqlite3");
        let key = "workflow-start:one:";
        let caller = "workflow-start:one:local";
        let other_caller = "workflow-start:one:other";
        let other_endpoint = "workflow-start:two:local";
        let now = 2 * RETENTION_MS;
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            for card in [caller, other_caller, other_endpoint, caller] {
                store
                    .record_workflow_start_refusal(notice(card, now))
                    .unwrap();
            }
            store
                .set_person_inbox_card_state(
                    "owner",
                    vec![caller.into(), other_endpoint.into()],
                    InboxCardChange::Snooze {
                        until_ms: Some(now + 3 * RETENTION_MS),
                    },
                    now,
                )
                .unwrap();
        }
        let store = DurableKernelStateStore::open(path).unwrap();
        let state = store
            .person_inbox_state("owner", now + 2 * RETENTION_MS)
            .unwrap();
        assert_eq!(state.notices.len(), 3);
        assert_eq!(
            state
                .notices
                .iter()
                .find(|n| n.card_id == caller)
                .unwrap()
                .folded,
            1
        );
        assert!(store.clear_workflow_start_refusals(key).unwrap());
        assert!(!store.clear_workflow_start_refusals(key).unwrap());
        let state = store.person_inbox_state("owner", now).unwrap();
        assert_eq!(state.notices[0].card_id, other_endpoint);
        assert_eq!(state.notices.len(), 1);
        assert!(!state.snoozes.contains_key(caller));
        assert!(state.snoozes.contains_key(other_endpoint));
    }

    #[test]
    fn the_inbox_state_survives_a_restart_and_a_muted_origin_s_notice_arrives_read() {
        let dir = crate::test_support::TestWorktree::new("person-inbox-state");
        let path = dir.path().join("state.sqlite3");
        let now = 10 * RETENTION_MS;
        {
            let store = DurableKernelStateStore::open(path.clone()).unwrap();
            store.record_person_notice(notice("n1", now)).unwrap();
            store
                .set_person_inbox_card_state(
                    "owner",
                    vec!["n1".into()],
                    InboxCardChange::Read { read: true },
                    now,
                )
                .unwrap();
            store
                .set_person_inbox_card_state(
                    "owner",
                    vec!["interaction-1".into()],
                    InboxCardChange::Snooze {
                        until_ms: Some(now + 3_600_000),
                    },
                    now,
                )
                .unwrap();
            let origin = PersonInboxOrigin {
                session_id: "room-1".into(),
                agent_id: "agent-1".into(),
            };
            store.set_person_inbox_mute("owner", origin, true).unwrap();
            assert!(
                store
                    .record_person_notice(notice("n2", now + 1))
                    .unwrap()
                    .read
            );
        }
        let store = DurableKernelStateStore::open(path.clone()).unwrap();
        let state = store.person_inbox_state("owner", now + 2).unwrap();
        assert_eq!(
            state
                .notices
                .iter()
                .map(|n| (n.card_id.as_str(), n.read))
                .collect::<Vec<_>>(),
            [("n2", true), ("n1", true)]
        );
        assert_eq!(state.snoozes.get("interaction-1"), Some(&(now + 3_600_000)));
        assert_eq!(state.mutes.len(), 1);
        assert!(
            store
                .person_inbox_state("owner", now + 3_600_000)
                .unwrap()
                .snoozes
                .is_empty(),
            "a snooze ends on time"
        );
        assert!(
            store.person_inbox_state("someone-else", now).unwrap() == PersonInboxState::default()
        );
    }

    #[test]
    fn notices_leave_after_seven_days_and_past_the_limit_the_oldest_read_goes_first() {
        let dir = crate::test_support::TestWorktree::new("person-inbox-limit");
        let store = DurableKernelStateStore::open(dir.path().join("state.sqlite3")).unwrap();
        let start = 10 * RETENTION_MS;
        store.record_person_notice(notice("old", start)).unwrap();
        store
            .record_person_notice(notice("unread", start + RETENTION_MS))
            .unwrap();
        store
            .record_person_notice(notice("read", start + RETENTION_MS + 1))
            .unwrap();
        store
            .set_person_inbox_card_state(
                "owner",
                vec!["read".into()],
                InboxCardChange::Read { read: true },
                start + RETENTION_MS + 1,
            )
            .unwrap();
        for index in 0..NOTICES_PER_OWNER - 1 {
            // One agent each, so the rate limit leaves every notice its own card.
            let mut sent = notice(
                &format!("n{index:04}"),
                start + RETENTION_MS + 2 + index as u64,
            );
            sent.agent_id = format!("agent-{index}");
            store.record_person_notice(sent).unwrap();
        }
        let state = store
            .person_inbox_state("owner", start + RETENTION_MS + 2_000)
            .unwrap();
        let ids = state
            .notices
            .iter()
            .map(|n| n.card_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), NOTICES_PER_OWNER as usize);
        assert!(!ids.contains(&"old"), "past retention");
        assert!(!ids.contains(&"read"), "the oldest read notice goes first");
        assert!(ids.contains(&"unread"));
    }
}
