//! Peers across kernels, receiving side: the agents of other kernels that follow an agent here,
//! and the outcomes owed to them. An outcome row is written in the same transaction as the
//! settlement it reports, so a turn end is never lost between the ledger and the outbox.
use super::*;

/// How many undelivered outcomes one subscriber kernel may hold; the oldest go first.
const OUTBOX_PER_KERNEL: i64 = 1_000;
/// An outcome nobody received in a day is no longer news.
const OUTBOX_TTL_MS: u64 = 24 * 60 * 60 * 1_000;
/// The occurrence of a followed agent or room that is gone; the follower's event says so.
pub(crate) const SOURCE_LOST_OCCURRENCE: &str = "source-lost-";

/// A follow of an agent reports every turn of its peer, so it outlives the turns it reports; only
/// the loss of the peer (`agent-gone-*`, `source-lost-*`) or an unfollow ends it. A follow of one
/// task ends with that task.
pub(super) fn outlives(
    tx: &Transaction<'_>,
    registration: &Registration,
    occurrence: &str,
) -> Result<bool, DaemonError> {
    let Some(ended) = occurrence.strip_prefix("task-terminal-") else {
        return Ok(false);
    };
    Ok(registration.peer
        && followed_task(tx, &registration.source_id)?.is_none_or(|task| task != ended))
}

/// The task a follow names by its task or prompt id; `None` when it follows an agent. A task id
/// names one task in every room, so a follow from another room finds it too.
pub(super) fn followed_task(
    tx: &Transaction<'_>,
    source: &str,
) -> Result<Option<String>, DaemonError> {
    tx.query_row(
        "SELECT task_id FROM agent_tasks WHERE task_id=?1 OR prompt_id=?1 ORDER BY task_id=?1 DESC LIMIT 1",
        params![source],
        |r| r.get(0),
    )
    .optional()
    .map_err(sql)
}

/// The live follows an agent made in any of its tasks: a follow outlives the turn that made it.
pub(super) fn held_by(
    db: &Connection,
    room: &str,
    agent: &str,
) -> Result<Vec<Registration>, DaemonError> {
    let mut q = db
        .prepare("SELECT r.payload FROM agent_registrations r JOIN agent_tasks t ON t.task_id=r.task_id WHERE t.room_id=?1 AND t.agent_id=?2 AND json_valid(r.payload) AND COALESCE(json_extract(r.payload,'$.peer'),0)=1 AND COALESCE(json_extract(r.payload,'$.live'),0)=1 ORDER BY r.rowid")
        .map_err(sql)?;
    let rows = q
        .query_map(params![room, agent], |r| r.get::<_, String>(0))
        .map_err(sql)?;
    rows.map(|row| decode(&row.map_err(sql)?)).collect()
}

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS peer_agent_followers(subscription_id TEXT PRIMARY KEY,subscriber_kernel_id TEXT NOT NULL,owner TEXT NOT NULL,target_room TEXT NOT NULL,target_agent TEXT NOT NULL,generation INTEGER NOT NULL,live INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS peer_agent_followers_target ON peer_agent_followers(target_room,target_agent);
        CREATE TABLE IF NOT EXISTS peer_agent_outbox(sequence INTEGER PRIMARY KEY AUTOINCREMENT,delivery_id TEXT NOT NULL UNIQUE,subscription_id TEXT NOT NULL,subscriber_kernel_id TEXT NOT NULL,owner TEXT NOT NULL,target_room TEXT NOT NULL,target_agent TEXT NOT NULL,occurrence_id TEXT NOT NULL,event TEXT NOT NULL,attempts INTEGER NOT NULL,next_attempt_at_ms INTEGER NOT NULL,expires_at_ms INTEGER NOT NULL);",
    )
    .map_err(sql)
}

/// One outcome owed to a follower on another kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PeerOutboxRow {
    pub delivery_id: String,
    pub subscription_id: String,
    pub subscriber_kernel_id: String,
    pub owner: String,
    pub target_room: String,
    pub target_agent: String,
    pub occurrence_id: String,
    pub event: crate::transport::relay_peer_agents::PeerEventKind,
    pub attempts: u32,
}

/// What a follow request did: `generation` is the one that now holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PeerFollowed {
    pub generation: u64,
}

pub(super) fn apply(tx: &Transaction<'_>, op: Operation) -> Result<Outcome, DaemonError> {
    match op {
        Operation::FollowPeer {
            kernel,
            owner,
            subscription,
            room,
            agent,
            generation,
        } => {
            let stored: Option<(String, u64)> = tx
                .query_row(
                    "SELECT subscriber_kernel_id,generation FROM peer_agent_followers WHERE subscription_id=?1",
                    params![subscription],
                    |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)),
                )
                .optional()
                .map_err(sql)?;
            match stored {
                // Another kernel's subscription id is not this kernel's to take over.
                Some((other, _)) if other != kernel => {
                    return Err(error("subscription belongs to another kernel"))
                }
                Some((_, held)) if held > generation => {
                    return Ok(Outcome::PeerFollowed(PeerFollowed { generation: held }))
                }
                _ => {}
            }
            tx.execute(
                "INSERT INTO peer_agent_followers VALUES(?1,?2,?3,?4,?5,?6,1) ON CONFLICT(subscription_id) DO UPDATE SET target_room=excluded.target_room,target_agent=excluded.target_agent,generation=excluded.generation,live=1",
                params![subscription, kernel, owner, room, agent, sql_integer(generation)?],
            )
            .map_err(sql)?;
            Ok(Outcome::PeerFollowed(PeerFollowed { generation }))
        }
        Operation::UnfollowPeer {
            kernel,
            subscription,
            generation,
        } => {
            // Only the kernel that follows ends its follow, and never with an older generation.
            tx.execute(
                "UPDATE peer_agent_followers SET live=0 WHERE subscription_id=?1 AND subscriber_kernel_id=?2 AND generation<=?3",
                params![subscription, kernel, sql_integer(generation)?],
            )
            .map_err(sql)?;
            Ok(Outcome::Saved)
        }
        Operation::PeerSourceLost { room, agent, now } => {
            for follower in live_followers(tx, &room, &agent)? {
                let occurrence = format!("{SOURCE_LOST_OCCURRENCE}{}", follower.subscription);
                owe(
                    tx,
                    &follower,
                    &occurrence,
                    crate::transport::relay_peer_agents::PeerEventKind::SourceLost,
                    now,
                )?;
                tx.execute(
                    "UPDATE peer_agent_followers SET live=0 WHERE subscription_id=?1",
                    params![follower.subscription],
                )
                .map_err(sql)?;
            }
            Ok(Outcome::Saved)
        }
        Operation::PeerOutboxDelivered { delivery } => {
            tx.execute(
                "DELETE FROM peer_agent_outbox WHERE delivery_id=?1",
                params![delivery],
            )
            .map_err(sql)?;
            Ok(Outcome::Saved)
        }
        Operation::PeerOutboxRetry { delivery, at, now } => {
            tx.execute(
                "UPDATE peer_agent_outbox SET attempts=attempts+1,next_attempt_at_ms=?2 WHERE delivery_id=?1",
                params![delivery, sql_integer(at)?],
            )
            .map_err(sql)?;
            tx.execute(
                "DELETE FROM peer_agent_outbox WHERE expires_at_ms<=?1",
                params![sql_integer(now)?],
            )
            .map_err(sql)?;
            Ok(Outcome::Saved)
        }
        _ => Err(error("not a peer follow operation")),
    }
}

/// A followed agent's turn ended: one outcome per live follower. The follow goes on; only the
/// loss of the agent or the follower's unfollow ends it.
pub(super) fn turn_ended(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
    occurrence: &str,
    success: bool,
    now: u64,
) -> Result<(), DaemonError> {
    if !occurrence.starts_with("task-terminal-") {
        return Ok(());
    }
    let event = if success {
        crate::transport::relay_peer_agents::PeerEventKind::TurnEnded
    } else {
        crate::transport::relay_peer_agents::PeerEventKind::TurnFailed
    };
    for follower in live_followers(tx, room, agent)? {
        owe(tx, &follower, occurrence, event, now)?;
    }
    Ok(())
}

struct Follower {
    subscription: String,
    kernel: String,
    owner: String,
    room: String,
    agent: String,
}

fn live_followers(
    tx: &Transaction<'_>,
    room: &str,
    agent: &str,
) -> Result<Vec<Follower>, DaemonError> {
    let mut q = tx
        .prepare("SELECT subscription_id,subscriber_kernel_id,owner FROM peer_agent_followers WHERE target_room=?1 AND target_agent=?2 AND live=1")
        .map_err(sql)?;
    let rows = q
        .query_map(params![room, agent], |r| {
            Ok(Follower {
                subscription: r.get(0)?,
                kernel: r.get(1)?,
                owner: r.get(2)?,
                room: room.into(),
                agent: agent.into(),
            })
        })
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    Ok(rows)
}

fn owe(
    tx: &Transaction<'_>,
    follower: &Follower,
    occurrence: &str,
    event: crate::transport::relay_peer_agents::PeerEventKind,
    now: u64,
) -> Result<(), DaemonError> {
    let event = serde_json::to_value(event)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(|| error("peer event kind"))?;
    tx.execute(
        "INSERT INTO peer_agent_outbox(delivery_id,subscription_id,subscriber_kernel_id,owner,target_room,target_agent,occurrence_id,event,attempts,next_attempt_at_ms,expires_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,0,?9,?10) ON CONFLICT(delivery_id) DO NOTHING",
        params![
            format!("{}:{occurrence}", follower.subscription),
            follower.subscription,
            follower.kernel,
            follower.owner,
            follower.room,
            follower.agent,
            occurrence,
            event,
            sql_integer(now)?,
            sql_integer(now.saturating_add(OUTBOX_TTL_MS))?,
        ],
    )
    .map_err(sql)?;
    tx.execute(
        "DELETE FROM peer_agent_outbox WHERE subscriber_kernel_id=?1 AND sequence NOT IN (SELECT sequence FROM peer_agent_outbox WHERE subscriber_kernel_id=?1 ORDER BY sequence DESC LIMIT ?2)",
        params![follower.kernel, OUTBOX_PER_KERNEL],
    )
    .map_err(sql)?;
    Ok(())
}

impl DurableKernelStateStore {
    /// Outcomes due by `due_by`, oldest first; ones expired at `now` are not news any more.
    pub(crate) fn peer_agent_outbox_due(
        &self,
        due_by: u64,
        now: u64,
    ) -> Result<Vec<PeerOutboxRow>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.peer_outbox")?;
        let mut q = db
            .prepare("SELECT delivery_id,subscription_id,subscriber_kernel_id,owner,target_room,target_agent,occurrence_id,event,attempts FROM peer_agent_outbox WHERE next_attempt_at_ms<=?1 AND expires_at_ms>?2 ORDER BY sequence LIMIT 256")
            .map_err(sql)?;
        let rows = q
            .query_map(params![sql_integer(due_by)?, sql_integer(now)?], |r| {
                Ok((
                    PeerOutboxRow {
                        delivery_id: r.get(0)?,
                        subscription_id: r.get(1)?,
                        subscriber_kernel_id: r.get(2)?,
                        owner: r.get(3)?,
                        target_room: r.get(4)?,
                        target_agent: r.get(5)?,
                        occurrence_id: r.get(6)?,
                        event: crate::transport::relay_peer_agents::PeerEventKind::TurnEnded,
                        attempts: r.get::<_, i64>(8)?.try_into().unwrap_or(u32::MAX),
                    },
                    r.get::<_, String>(7)?,
                ))
            })
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        rows.into_iter()
            .map(|(mut row, event)| {
                row.event = serde_json::from_value(serde_json::Value::String(event))
                    .map_err(|_| error("corrupt peer outcome event"))?;
                Ok(row)
            })
            .collect()
    }

    /// The agents here that agents of other kernels still follow.
    pub(crate) fn peer_followed_agents(&self) -> Result<Vec<(String, String)>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.peer_followers")?;
        let mut q = db
            .prepare(
                "SELECT DISTINCT target_room,target_agent FROM peer_agent_followers WHERE live=1",
            )
            .map_err(sql)?;
        let rows = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        Ok(rows)
    }

    /// A registration by its id, whichever task holds it.
    pub(crate) fn agent_registration(&self, id: &str) -> Result<Option<Registration>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.registration")?;
        db.query_row(
            "SELECT payload FROM agent_registrations WHERE id=?1",
            params![id],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(sql)?
        .map(|payload| decode(&payload))
        .transpose()
    }

    /// The live follows held here of agents in other rooms, here or on other kernels.
    pub(crate) fn live_peer_registrations(&self) -> Result<Vec<Registration>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.registration")?;
        let mut q = db
            .prepare("SELECT payload FROM agent_registrations")
            .map_err(sql)?;
        let payloads = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        let mut live = Vec::new();
        for payload in payloads {
            let registration: Registration = decode(&payload)?;
            if registration.peer && registration.live {
                live.push(registration);
            }
        }
        Ok(live)
    }

    /// Whether `occurrence` of `source` was already recorded in `room`.
    pub(crate) fn agent_source_occurred(
        &self,
        room: &str,
        source: &str,
        occurrence: &str,
    ) -> Result<bool, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.source_occurrences")?;
        db.query_row(
            "SELECT 1 FROM agent_source_occurrences WHERE room_id=?1 AND source_id=?2 AND occurrence_id=?3",
            params![room, source, occurrence],
            |_| Ok(()),
        )
        .optional()
        .map(|found| found.is_some())
        .map_err(sql)
    }

    /// The newest source occurrence so far: a follow that starts now hears only of later ones.
    pub(crate) fn agent_source_cursor(&self) -> Result<u64, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.source_occurrences")?;
        let sequence: i64 = db
            .query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM agent_source_occurrences",
                [],
                |r| r.get(0),
            )
            .map_err(sql)?;
        u64::try_from(sequence).map_err(|_| error("corrupt source sequence"))
    }
}
