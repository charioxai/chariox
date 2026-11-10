//! MP-08/MP-10/MP-11: Cloud enrollment adopts the OS owner's gate work in place.
//! Every entry keeps its disposition, sequence, revisions and deadline; only the
//! owner and the owner-derived identities move to the enrolled user.
use super::*;

pub(super) fn enroll_owner_in(
    tx: &Transaction<'_>,
    session: &str,
    old: &str,
    new: &str,
) -> Result<()> {
    // Modules are owner-wide content-addressed storage: identical digests merge.
    tx.execute("INSERT OR IGNORE INTO workflow_gate_modules SELECT ?2,digest,component,created_at_ms FROM workflow_gate_modules WHERE owner_id=?1",params![old,new]).map_err(sql)?;
    tx.execute("DELETE FROM workflow_gate_modules WHERE owner_id=?1", [old])
        .map_err(sql)?;
    tx.execute(
        "UPDATE workflow_gate_lanes SET owner_id=?3 WHERE owner_id=?1 AND session_id=?2",
        params![old, session, new],
    )
    .map_err(sql)?;
    tx.execute(
        "UPDATE workflow_gate_state SET owner_id=?3 WHERE owner_id=?1 AND session_id=?2",
        params![old, session, new],
    )
    .map_err(sql)?;
    let entries = ids(
        tx,
        "SELECT entry_id FROM workflow_gate_entries WHERE owner_id=?1 AND session_id=?2 ORDER BY sequence",
        params![old, session],
    )?;
    for id in &entries {
        let body: String = tx
            .query_row(
                "SELECT body_json FROM workflow_gate_entries WHERE entry_id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(sql)?;
        // A compacted entry keeps only its owner-free summary.
        if body.is_empty() {
            tx.execute(
                "UPDATE workflow_gate_entries SET owner_id=?2 WHERE entry_id=?1",
                params![id, new],
            )
            .map_err(sql)?;
            continue;
        }
        let mut stored: Stored = decode(&body)?;
        enroll_admission(&mut stored.admission, old, new)?;
        tx.execute(
            "UPDATE workflow_gate_entries SET owner_id=?2,content_digest=?3,body_json=?4 WHERE entry_id=?1",
            params![id, new, admission::content_digest(&stored.admission)?, encode(&stored)?],
        )
        .map_err(sql)?;
    }
    // An open batch is found again by its owner-derived scope.
    for (group, leader) in rows(tx, "SELECT b.group_id,b.entry_id FROM workflow_gate_batches b JOIN workflow_gate_entries e ON e.entry_id=b.entry_id WHERE b.closed=0 AND e.owner_id=?1 AND e.session_id=?2", params![new, session])? {
        let Some(key) = load(tx, &leader)?.and_then(|s| s.batch).map(|b| b.key) else {
            return Err(error("gate batch aggregate missing"));
        };
        let member: String = tx
            .query_row(
                "SELECT entry_id FROM workflow_gate_members WHERE group_id=?1 LIMIT 1",
                [&group],
                |r| r.get(0),
            )
            .map_err(sql)?;
        let member = load(tx, &member)?.ok_or_else(|| error("gate batch member missing"))?;
        tx.execute(
            "UPDATE workflow_gate_batches SET scope_sha256=?2 WHERE group_id=?1",
            params![group, decisions::batch_scope(&member, &key)?],
        )
        .map_err(sql)?;
    }
    Ok(())
}

fn enroll_admission(a: &mut Admission, old: &str, new: &str) -> Result<()> {
    if a.owner != old {
        return Ok(());
    }
    a.owner = new.into();
    // A collaborator's caller identity is theirs and stays unchanged.
    let creator = a.template.created_by_agent_id();
    if a.caller_sha256 == admission::caller_sha256(old, creator)? {
        a.caller_sha256 = admission::caller_sha256(new, creator)?;
    }
    Ok(())
}

fn rows(db: &Connection, query: &str, p: impl rusqlite::Params) -> Result<Vec<(String, String)>> {
    let mut q = db.prepare(query).map_err(sql)?;
    let rows = q
        .query_map(p, |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(sql)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql)
}
