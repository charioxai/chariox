//! MP-08 / MP-09 / MP-10 / MP-11: bounded inboxes retain a gap per source kernel.
use super::*;
use std::collections::BTreeMap;
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
    let count: i64 = tx
        .query_row(
            "SELECT count(*) FROM agent_inbox WHERE room_id='' AND agent_id=?1",
            [&part.id],
            |row| row.get(0),
        )
        .map_err(sql)?;
    let count = usize::try_from(count).map_err(|_| error("invalid inbox row count"))?;
    if count <= limit {
        return Ok(());
    }
    let mut drop_count = count - limit + 1;
    let (groups, last) = loop {
        let mut query=tx.prepare("SELECT sequence,payload FROM agent_inbox WHERE room_id='' AND agent_id=?1 ORDER BY sequence LIMIT ?2").map_err(sql)?;
        let rows = query
            .query_map(params![part.id, drop_count as i64], |row| {
                Ok((cursor_from_row(row)?, row.get::<_, String>(1)?))
            })
            .map_err(sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql)?;
        let mut groups: BTreeMap<String, Dropped> = BTreeMap::new();
        let last = rows
            .last()
            .ok_or_else(|| error("inbox retention range missing"))?
            .0;
        for (sequence, payload) in rows {
            let event: OutsideEvent = decode(&payload)?;
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
        if retained <= limit {
            break (groups, last);
        }
        if drop_count == count {
            return Err(error(
                "test inbox limit must retain a gap for every followed kernel",
            ));
        }
        drop_count = (drop_count + retained - limit).min(count);
    };
    tx.execute(
        "DELETE FROM agent_inbox WHERE room_id='' AND agent_id=?1 AND sequence<=?2",
        params![part.id, sql_integer(last)?],
    )
    .map_err(sql)?;
    let oldest = groups
        .values()
        .map(|group| group.sequence)
        .min()
        .ok_or_else(|| error("inbox retention range missing"))?;
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
    part.floor = part.floor.max(oldest.saturating_sub(1));
    Ok(())
}
