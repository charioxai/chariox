//! Peers a room agent created in other rooms (on this kernel or another) and the agents that
//! made them. The link is written when the peer is created and removed when the peer has been
//! deleted after its creator ended, so a restart in between still finishes the deletes.
use super::*;

pub(super) fn initialize(db: &Connection) -> Result<(), DaemonError> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS created_peers(creator_room TEXT NOT NULL,creator_agent TEXT NOT NULL,kernel TEXT NOT NULL,peer TEXT NOT NULL,created_at_ms INTEGER NOT NULL,PRIMARY KEY(creator_agent,kernel,peer));",
    )
    .map_err(sql)
}

/// One peer an agent created. `kernel` is empty for a room of this kernel; `peer` is the new
/// agent's id there, or the spawn request id for a peer on another kernel, whose agent id and
/// room the creator may never have learned (a lost answer, a room renamed since).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CreatedPeer {
    pub creator_room: String,
    pub creator_agent: String,
    pub kernel: String,
    pub peer: String,
    pub created_at_ms: u64,
}

pub(super) fn apply(tx: &Transaction<'_>, op: Operation) -> Result<Outcome, DaemonError> {
    match op {
        Operation::RecordCreatedPeer { peer } => {
            let inserted = tx
                .execute(
                    "INSERT OR IGNORE INTO created_peers VALUES(?1,?2,?3,?4,?5)",
                    params![
                        peer.creator_room,
                        peer.creator_agent,
                        peer.kernel,
                        peer.peer,
                        sql_integer(peer.created_at_ms)?
                    ],
                )
                .map_err(sql)?;
            Ok(Outcome::CreatedPeerRecorded {
                inserted: inserted > 0,
            })
        }
        Operation::ReleaseCreatedPeer { peer } => {
            tx.execute(
                "DELETE FROM created_peers WHERE creator_agent=?1 AND kernel=?2 AND peer=?3",
                params![peer.creator_agent, peer.kernel, peer.peer],
            )
            .map_err(sql)?;
            Ok(Outcome::Saved)
        }
        _ => Err(error("not a created peer operation")),
    }
}

impl DurableKernelStateStore {
    /// Every link not yet released, oldest first.
    pub(crate) fn created_peers(&self) -> Result<Vec<CreatedPeer>, DaemonError> {
        let db = self.lock_connection("agent.lifecycle.created_peers")?;
        let mut q = db
            .prepare("SELECT creator_room,creator_agent,kernel,peer,created_at_ms FROM created_peers ORDER BY created_at_ms,creator_agent,peer")
            .map_err(sql)?;
        let rows = q
            .query_map([], |r| {
                Ok(CreatedPeer {
                    creator_room: r.get(0)?,
                    creator_agent: r.get(1)?,
                    kernel: r.get(2)?,
                    peer: r.get(3)?,
                    created_at_ms: r.get::<_, i64>(4)?.try_into().unwrap_or(0),
                })
            })
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        Ok(rows)
    }
}
