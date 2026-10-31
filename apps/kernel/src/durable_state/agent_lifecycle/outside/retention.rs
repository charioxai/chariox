//! MP-08 / MP-09 / MP-10 / MP-11: bounded inboxes retain a gap per source kernel.
use super::*;
use std::collections::BTreeMap;

/// How long an ended part keeps its rows and its ending receipt. Its holder has read the
/// ending by then, or never will.
const ENDED_RETENTION_MS: u64 = 24 * 60 * 60 * 1000;

struct Dropped {
    sequence: u64,
    event: OutsideEvent,
    from: u64,
    to: u64,
    count: u64,
}

pub(in crate::durable_state::agent_lifecycle) fn bound(
    tx: &Transaction<'_>,
    part: &mut Part,
    limit: usize,
) -> Result<(), DaemonError> {
    // Only appends after the last count add rows, and each takes a higher inbox sequence.
    if part.counted.is_some_and(|counted| {
        counted.rows + part.latest.saturating_sub(counted.at) <= limit as u64
    }) {
        return Ok(());
    }
    let count: i64 = tx
        .query_row(
            "SELECT count(*) FROM agent_inbox WHERE room_id='' AND agent_id=?1",
            [&part.id],
            |row| row.get(0),
        )
        .map_err(sql)?;
    let count = usize::try_from(count).map_err(|_| error("invalid inbox row count"))?;
    if count <= limit {
        part.counted = Some(CountedInbox {
            at: part.latest,
            rows: count as u64,
        });
        return Ok(());
    }
    // A full inbox is trimmed with headroom, so it is not counted and trimmed again on
    // every following event.
    let target = limit - limit / 16;
    let mut drop_count = count - target + 1;
    let (groups, last, health_floor, retained) = loop {
        let mut query=tx.prepare("SELECT sequence,payload FROM agent_inbox WHERE room_id='' AND agent_id=?1 ORDER BY sequence LIMIT ?2").map_err(sql)?;
        let rows = query
            .query_map(params![part.id, drop_count as i64], |row| {
                Ok((cursor_from_row(row)?, row.get::<_, String>(1)?))
            })
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        let mut groups: BTreeMap<String, Dropped> = BTreeMap::new();
        let mut health_only = BTreeMap::new();
        let last = rows
            .last()
            .ok_or_else(|| error("inbox retention range missing"))?
            .0;
        for (sequence, payload) in rows {
            let event: OutsideEvent = decode(&payload)?;
            // Health is observed by the home kernel, outside the source's
            // sequence space. It advances cursor coverage, not source loss.
            if matches!(event.value, OutsideEventValue::KernelHealth { .. }) {
                if let Some(group) = groups.get_mut(&event.kernel) {
                    group.sequence = sequence;
                } else {
                    health_only.insert(event.kernel, sequence);
                }
                continue;
            }
            health_only.remove(&event.kernel);
            let source = if event.cursor == 0 {
                sequence
            } else {
                event.cursor
            };
            let (from, to, count) = match event.value {
                OutsideEventValue::Gap {
                    dropped, from, to, ..
                } => (from, to, dropped),
                _ => (source, source, 1),
            };
            match groups.get_mut(&event.kernel) {
                Some(group) => {
                    group.sequence = sequence;
                    group.from = group.from.min(from);
                    group.to = group.to.max(to);
                    group.count += count;
                }
                None => {
                    groups.insert(
                        event.kernel.clone(),
                        Dropped {
                            sequence,
                            event,
                            from,
                            to,
                            count,
                        },
                    );
                }
            }
        }
        let retained = count - drop_count + groups.len();
        if retained <= target {
            // A health-only kernel has no source gap to carry its lost transition.
            let health_floor = health_only.into_values().max().unwrap_or(0);
            break (groups, last, health_floor, retained);
        }
        if drop_count == count {
            return Err(error(
                "test inbox limit must retain a gap for every followed kernel",
            ));
        }
        drop_count = (drop_count + retained - target).min(count);
    };
    tx.execute(
        "DELETE FROM agent_inbox WHERE room_id='' AND agent_id=?1 AND sequence<=?2",
        params![part.id, sql_integer(last)?],
    )
    .map_err(sql)?;
    let floor = groups
        .values()
        .map(|group| group.sequence.saturating_sub(1))
        .min()
        .unwrap_or(last)
        .max(health_floor);
    for group in groups.into_values() {
        let mut event = group.event;
        event.cursor = group.sequence;
        event.value = OutsideEventValue::Gap {
            kernel: event.kernel.clone(),
            dropped: group.count,
            from: group.from,
            to: group.to,
        };
        event.summary = format!(
            "{} events dropped; resync this kernel's followed targets",
            group.count
        );
        event.agent_id = None;
        event.refs = Default::default();
        // A gap occupies the source's last removed sequence, before every retained event.
        tx.execute("INSERT INTO agent_inbox(sequence,room_id,agent_id,source_id,occurrence_id,payload) VALUES(?1,'',?2,?3,?4,?5)",params![sql_integer(group.sequence)?,part.id,event.kernel,format!("gap-{}",group.sequence),encode(&event)?]).map_err(sql)?;
    }
    part.floor = part.floor.max(floor);
    part.counted = Some(CountedInbox {
        at: part.latest,
        rows: retained as u64,
    });
    Ok(())
}

/// Deletes the parts that ended more than a day ago, with their unread rows. A home part
/// whose remote cleanup is still owed stays until its pump has finished it.
pub(in crate::durable_state::agent_lifecycle) fn prune_ended(
    tx: &Transaction<'_>,
    now_ms: u64,
) -> Result<(), DaemonError> {
    for part in parts(tx)? {
        let expired = part
            .ended_at_ms
            .is_some_and(|at| now_ms.saturating_sub(at) >= ENDED_RETENTION_MS);
        if part.ended.is_none() || !expired || !part.cleanup_targets.is_empty() {
            continue;
        }
        tx.execute(
            "DELETE FROM agent_inbox WHERE room_id='' AND agent_id=?1",
            [&part.id],
        )
        .map_err(sql)?;
        tx.execute(
            "DELETE FROM agent_outside_subscriptions WHERE id=?1",
            [&part.id],
        )
        .map_err(sql)?;
    }
    Ok(())
}
