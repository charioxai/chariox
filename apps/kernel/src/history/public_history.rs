//! MP-08 / MP-10 / MP-11, A09: sanitized source and rebuildable room-scoped FTS.
//! No raw-history backfill. Only the runtime projection can create public rows.
use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};

fn read_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

// Version 1 may contain MCP records admitted without their private server
// identity; version 2 indexed streamed deltas and allowed unknown tools. The
// existing version fence removes those projections on upgrade.
pub(crate) const PUBLIC_HISTORY_VERSION: u32 = 3;
// Internal projection input only; never serialized into operational events.
pub(crate) const PUBLIC_HISTORY_BATCH_REFS: &str = "_public_history_batch_refs";
pub(crate) type PublicHistoryProjector =
    dyn Fn(&HistoryEvent) -> Option<PublicHistoryDocument> + Send + Sync;

#[derive(Default)]
pub(super) struct ProjectorSlot(
    pub Mutex<Option<Arc<PublicHistoryProjector>>>,
    Mutex<Option<Arc<PublicHistoryBoundary>>>,
);
type PublicHistoryBoundary = dyn Fn(&HistoryEvent) -> usize + Send + Sync;
impl ProjectorSlot {
    fn boundary_bytes(&self, event: &HistoryEvent) -> usize {
        self.1
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .map_or(usize::MAX, |boundary| boundary(event))
    }
    pub(super) fn project(&self, event: &HistoryEvent) -> Option<PublicHistoryDocument> {
        let projector = self.0.lock().ok()?.clone()?;
        projector(event)
    }
}
impl std::fmt::Debug for ProjectorSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PublicHistoryProjector")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PublicHistoryDocument {
    pub event_ref: String,
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub owner_user_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub kind: HistoryEventKind,
    pub turn_id: Option<String>,
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PublicHistoryCoverage {
    pub index_version: u32,
    pub redaction_version: u32,
    pub indexed_events: u64,
    pub excluded_events: u64,
    pub through_sequence: u64,
    pub complete: bool,
    pub rebuilding: bool,
    pub rebuild_cursor: u64,
    pub retention_gap_events: u64,
    pub truncated_events: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PublicHistoryHit {
    pub event_ref: String,
    pub sequence: u64,
    pub agent_id: String,
    pub kind: String,
    pub snippet: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PublicHistorySearchResult {
    pub hits: Vec<PublicHistoryHit>,
    pub next_cursor: Option<String>,
    pub coverage: PublicHistoryCoverage,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    binding: String,
    revision: u64,
    through: u64,
    offset: u32,
}

fn invalid_search() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "history.public",
        message: "Invalid history query, reference or stale cursor".into(),
    }
}

impl OperationalHistoryStore {
    pub(crate) fn set_public_history_projector(&self, projector: Arc<PublicHistoryProjector>) {
        *self
            .public_history_projector
            .0
            .lock()
            .expect("public history projector lock") = Some(projector);
    }

    pub(crate) fn set_public_history_boundary(&self, boundary: Arc<PublicHistoryBoundary>) {
        *self
            .public_history_projector
            .1
            .lock()
            .expect("public history boundary lock") = Some(boundary);
    }

    pub(crate) fn lock_public_history(&self) -> Result<MutexGuard<'_, ()>, DaemonError> {
        self.public_history_lock
            .lock()
            .map_err(|_| invalid_search())
    }

    // MP-08 / MP-10 / MP-11: caller holds the projection fence and has drained
    // pending writes. Evaluate only sanitized messages, outside SQLite locks.
    pub(crate) fn retain_public_history_locked(
        &self,
        room: Option<&str>,
        candidates: Option<&[zeroize::Zeroizing<String>]>,
        retain: &dyn Fn(&PublicHistoryDocument) -> bool,
    ) -> Result<(), DaemonError> {
        // Search serialized variants (including escaped field names/values)
        // before deserialization. Unknown registry state scans conservatively.
        let mut query = String::from("SELECT sequence,document_json FROM public_history WHERE (?1 IS NULL OR session_id=?1) AND sequence>?2");
        if let Some(candidates) = candidates {
            query.push_str(" AND (");
            for index in 0..candidates.len() {
                if index > 0 {
                    query.push_str(" OR ");
                }
                query.push_str(&format!("instr(document_json,?{})>0", index + 3));
            }
            if candidates.is_empty() {
                query.push('0');
            }
            query.push(')');
        }
        query.push_str(" ORDER BY sequence LIMIT 128");
        let mut through = -1_i64;
        let mut changed = false;
        loop {
            let rows = {
                let connection = self.lock_read_connection(room)?;
                let mut values: Vec<rusqlite::types::Value> = vec![
                    room.map_or(rusqlite::types::Value::Null, |room| room.to_owned().into()),
                    through.into(),
                ];
                if let Some(candidates) = candidates {
                    values.extend(
                        candidates
                            .iter()
                            .map(|value| rusqlite::types::Value::Text(value.to_string())),
                    );
                }
                let mut statement = connection.prepare(&query).map_err(public_error)?;
                let rows = statement
                    .query_map(rusqlite::params_from_iter(values.iter()), |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(public_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(public_error)?;
                rows
            };
            let Some((last, _)) = rows.last() else {
                break;
            };
            through = *last;
            let removed: Vec<_> = rows
                .into_iter()
                .filter_map(|(sequence, json)| {
                    serde_json::from_str(&json)
                        .ok()
                        .filter(retain)
                        .is_none()
                        .then_some(sequence)
                })
                .collect();
            if removed.is_empty() {
                continue;
            }
            changed = true;
            let mut connection = self.connection.lock().map_err(|_| invalid_search())?;
            let tx = connection.transaction().map_err(public_error)?;
            for sequence in removed {
                tx.execute(
                    "INSERT INTO public_history_revision(session_id,revision)
                    SELECT session_id,1 FROM public_history WHERE sequence=?1
                    ON CONFLICT(session_id) DO UPDATE SET revision=revision+1",
                    [sequence],
                )
                .map_err(public_error)?;
                tx.execute("DELETE FROM public_history WHERE sequence=?1", [sequence])
                    .map_err(public_error)?;
            }
            tx.commit().map_err(public_error)?;
        }
        if changed {
            let connection = self.connection.lock().map_err(|_| invalid_search())?;
            checkpoint_public_history(&connection, room);
        }
        Ok(())
    }

    // Caller holds the public projection lock through the observation mutation.
    // This also fences in-flight appends before changing secret provenance.
    pub(crate) fn invalidate_public_history_locked(
        &self,
        room: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.flush_public_history_appends_locked()?;
        let mut connection = self.connection.lock().map_err(|_| invalid_search())?;
        let tx = connection.transaction().map_err(public_error)?;
        tx.execute(
            "DELETE FROM public_history WHERE (?1 IS NULL OR session_id=?1)",
            [room],
        )
        .map_err(public_error)?;
        tx.execute(
            "INSERT INTO public_history_revision(session_id,revision) SELECT DISTINCT session_id,1 FROM history_events WHERE session_id IS NOT NULL AND (?1 IS NULL OR session_id=?1)
            ON CONFLICT(session_id) DO UPDATE SET revision=revision+1",
            [room],
        )
        .map_err(public_error)?;
        tx.commit().map_err(public_error)?;
        // SQLite + FTS secure-delete erase obsolete tokens; truncate the WAL too.
        // The deletion is committed: an active reader only delays truncation
        // until a later checkpoint, so it must not fail the protected mutation.
        checkpoint_public_history(&connection, room);
        Ok(())
    }

    pub(crate) fn search_public_history_locked(
        &self,
        owner: &str,
        room: &str,
        agent: Option<&str>,
        query: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<PublicHistorySearchResult, DaemonError> {
        if query.len() > 1024 || query.trim().is_empty() || owner.is_empty() || room.is_empty() {
            return Err(invalid_search());
        }
        // Literal token search, never user-supplied FTS operators/column selectors.
        let terms: Vec<_> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect();
        if terms.is_empty() || terms.len() > 32 {
            return Err(invalid_search());
        }
        let expression = terms
            .iter()
            .map(|s| format!("\"{}\"", s))
            .collect::<Vec<_>>()
            .join(" AND ");
        let binding = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(owner, room, agent, query)).map_err(|_| invalid_search())?
            )
        );
        self.advance_public_history_rebuild_locked(256)?;
        let connection = self.lock_read_connection(Some(room))?;
        let revision = public_revision(&connection, room)?;
        let coverage = coverage(&connection, owner, room)?;
        let (through, offset) = if let Some(cursor) = cursor {
            if cursor.len() > 2048 {
                return Err(invalid_search());
            }
            let decoded: Cursor = serde_json::from_slice(
                &URL_SAFE_NO_PAD
                    .decode(cursor)
                    .map_err(|_| invalid_search())?,
            )
            .map_err(|_| invalid_search())?;
            if decoded.binding != binding
                || decoded.revision != revision
                || decoded.offset > 100_000
                || decoded.through > coverage.through_sequence
            {
                return Err(invalid_search());
            }
            (decoded.through, decoded.offset)
        } else {
            (coverage.through_sequence, 0)
        };
        let limit = limit.clamp(1, 50);
        // Scope is joined INSIDE the query, before limit/snippets. Chronological
        // ranking avoids BM25 corpus statistics from other rooms. No global count.
        let mut statement = connection.prepare(
            "SELECT p.event_ref,p.sequence,p.agent_id,p.kind,snippet(public_history_fts,0,'','',' … ',32)
             FROM public_history_fts JOIN public_history p ON p.sequence=public_history_fts.rowid
             JOIN history_events h ON h.event_id=p.event_ref AND h.sequence=p.sequence
             WHERE public_history_fts MATCH ?1 AND p.owner_user_id=?2 AND p.session_id=?3
               AND (?4 IS NULL OR p.agent_id=?4) AND p.sequence<=?5
             ORDER BY p.sequence DESC LIMIT ?6 OFFSET ?7"
        ).map_err(public_error)?;
        let mut hits = statement
            .query_map(
                params![
                    expression,
                    owner,
                    room,
                    agent,
                    through as i64,
                    limit as i64 + 1,
                    offset
                ],
                |r| {
                    Ok(PublicHistoryHit {
                        event_ref: r.get(0)?,
                        sequence: read_u64(r, 1)?,
                        agent_id: r.get(2)?,
                        kind: r.get(3)?,
                        snippet: r.get::<_, String>(4)?.chars().take(512).collect(),
                    })
                },
            )
            .map_err(public_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(public_error)?;
        let next_cursor = if hits.len() > limit && (cursor.is_some() || !coverage.rebuilding) {
            hits.truncate(limit);
            Some(
                URL_SAFE_NO_PAD.encode(
                    serde_json::to_vec(&Cursor {
                        binding,
                        revision,
                        through,
                        offset: offset + limit as u32,
                    })
                    .map_err(|_| invalid_search())?,
                ),
            )
        } else {
            None
        };
        hits.truncate(limit);
        Ok(PublicHistorySearchResult {
            hits,
            next_cursor,
            coverage,
        })
    }

    pub(crate) fn public_history_revision_locked(&self, room: &str) -> Result<u64, DaemonError> {
        let connection = self.lock_read_connection(Some(room))?;
        public_revision(&connection, room)
    }

    pub(crate) fn read_public_history_locked(
        &self,
        owner: &str,
        room: &str,
        event_ref: &str,
    ) -> Result<Option<PublicHistoryDocument>, DaemonError> {
        if event_ref.len() > 256 || owner.is_empty() || room.is_empty() {
            return Err(invalid_search());
        }
        let connection = self.lock_read_connection(Some(room))?;
        read_public_document(&connection, owner, room, event_ref)
    }

    pub(crate) fn public_history_turn_locked(
        &self,
        owner: &str,
        room: &str,
        agent: &str,
        turn_ref: Option<&str>,
        turns_back: usize,
        limit: usize,
    ) -> Result<Vec<PublicHistoryDocument>, DaemonError> {
        let offset = if turn_ref.is_some() {
            0
        } else {
            i64::try_from(turns_back).map_err(|_| invalid_search())?
        };
        let connection = self.lock_read_connection(Some(room))?;
        // MP-08 / MP-10 / MP-11: one kernel prompt may have a different
        // provider-native turn ID. Accept either reference, select the logical
        // prompt first, then bound its messages in this one query.
        let mut statement = connection.prepare(
            "WITH selected_turn AS (
                SELECT COALESCE(h.prompt_id,json_extract(p.document_json,'$.turn_id')) AS turn_id
                FROM public_history p JOIN history_events h ON h.event_id=p.event_ref AND h.sequence=p.sequence
                WHERE p.owner_user_id=?1 AND p.session_id=?2 AND p.agent_id=?3
                  AND (?4 IS NULL OR COALESCE(h.prompt_id,json_extract(p.document_json,'$.turn_id'))=?4 OR h.turn_id=?4)
                GROUP BY COALESCE(h.prompt_id,json_extract(p.document_json,'$.turn_id')) ORDER BY max(p.sequence) DESC LIMIT 1 OFFSET ?5
             )
             SELECT p.document_json,h.prompt_id
             FROM public_history p
             JOIN history_events h ON h.event_id=p.event_ref AND h.sequence=p.sequence
             JOIN selected_turn t ON COALESCE(h.prompt_id,json_extract(p.document_json,'$.turn_id')) IS t.turn_id
             WHERE p.owner_user_id=?1 AND p.session_id=?2 AND p.agent_id=?3
             ORDER BY p.sequence DESC LIMIT ?6"
        ).map_err(public_error)?;
        let rows = statement
            .query_map(
                params![
                    owner,
                    room,
                    agent,
                    turn_ref,
                    offset,
                    limit.clamp(1, 200) as i64
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get(1)?)),
            )
            .map_err(public_error)?;
        let documents = rows
            .map(|row| {
                let (json, prompt) = row.map_err(public_error)?;
                public_document(&json, prompt)
            })
            .collect();
        documents
    }

    /// Start/resume a bounded rebuild from sanitized source, never raw rows.
    #[cfg(test)]
    pub(crate) fn begin_public_history_rebuild(&self) -> Result<(), DaemonError> {
        let _guard = self.lock_public_history()?;
        let mut connection = self.connection.lock().map_err(|_| invalid_search())?;
        let tx = connection.transaction().map_err(public_error)?;
        tx.execute_batch("INSERT INTO public_history_fts(public_history_fts) VALUES('delete-all'); UPDATE public_history_build SET cursor=0,complete=0,epoch=epoch+1;").map_err(public_error)?;
        tx.commit().map_err(public_error)
    }

    pub(crate) fn advance_public_history_rebuild_locked(
        &self,
        limit: usize,
    ) -> Result<(), DaemonError> {
        let mut connection = self.connection.lock().map_err(|_| invalid_search())?;
        let tx = connection.transaction().map_err(public_error)?;
        let (cursor, complete): (u64, bool) = tx
            .query_row(
                "SELECT cursor,complete FROM public_history_build",
                [],
                |r| Ok((read_u64(r, 0)?, r.get(1)?)),
            )
            .map_err(public_error)?;
        if complete {
            return Ok(());
        }
        let mut statement=tx.prepare("SELECT sequence,text FROM public_history WHERE sequence>?1 ORDER BY sequence LIMIT ?2").map_err(public_error)?;
        let rows = statement
            .query_map(
                params![cursor as i64, (limit.clamp(1, 1000) + 1) as i64],
                |r| Ok((read_u64(r, 0)?, r.get::<_, String>(1)?)),
            )
            .map_err(public_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(public_error)?;
        drop(statement);
        let finished = rows.len() <= limit.clamp(1, 1000);
        let mut through = cursor;
        for (seq, text) in rows.into_iter().take(limit.clamp(1, 1000)) {
            tx.execute(
                "INSERT INTO public_history_fts(rowid,text) VALUES(?1,?2)",
                params![seq as i64, text],
            )
            .map_err(public_error)?;
            through = seq;
        }
        tx.execute(
            "UPDATE public_history_build SET cursor=?1,complete=?2",
            params![through as i64, finished],
        )
        .map_err(public_error)?;
        tx.commit().map_err(public_error)
    }
}

fn checkpoint_public_history(connection: &Connection, room: Option<&str>) {
    if connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
        r.get::<_, i64>(0)
    }) != Ok(0)
    {
        crate::logging::warn_with_fields(
            "daemon.history",
            "MP-08/MP-10/MP-11: committed public history deletion; WAL truncation deferred",
            serde_json::json!({ "room": room }),
        );
    }
}

// MP-08 / MP-10 / MP-11: any projected event resolves to its sanitized message.
// Raw history contributes only the canonical prompt identity, never content.
fn read_public_document(
    connection: &Connection,
    owner: &str,
    room: &str,
    reference: &str,
) -> Result<Option<PublicHistoryDocument>, DaemonError> {
    connection
        .query_row(
            "SELECT p.document_json,h.prompt_id FROM public_history_member m
             JOIN public_history p ON p.sequence=m.sequence
             JOIN history_events h ON h.event_id=p.event_ref AND h.sequence=p.sequence
             WHERE m.event_ref=?3 AND p.owner_user_id=?1 AND p.session_id=?2",
            params![owner, room, reference],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(public_error)?
        .map(|(json, prompt)| public_document(&json, prompt))
        .transpose()
}

fn public_document(
    json: &str,
    prompt: Option<String>,
) -> Result<PublicHistoryDocument, DaemonError> {
    let mut document: PublicHistoryDocument =
        serde_json::from_str(json).map_err(|_| invalid_search())?;
    document.turn_id = prompt.or(document.turn_id);
    Ok(document)
}

fn public_error(error: rusqlite::Error) -> DaemonError {
    operational_history_error("public history index", error)
}

fn public_revision(connection: &Connection, room: &str) -> Result<u64, DaemonError> {
    connection.query_row("SELECT COALESCE((SELECT revision FROM public_history_revision WHERE session_id=?1),0)+(SELECT epoch FROM public_history_build)", [room], |r| read_u64(r,0)).map_err(public_error)
}

fn coverage(
    connection: &Connection,
    owner: &str,
    room: &str,
) -> Result<PublicHistoryCoverage, DaemonError> {
    let (build_cursor, globally_finished): (u64, bool) = connection
        .query_row(
            "SELECT cursor,complete FROM public_history_build",
            [],
            |r| Ok((read_u64(r, 0)?, r.get(1)?)),
        )
        .map_err(public_error)?;
    // Build bookkeeping is global internally; expose only this authorized room's
    // progress/counts, never another room's cursor or remaining document count.
    let (indexed_events, through_sequence, rebuild_cursor, pending): (u64,u64,u64,u64) = connection.query_row(
        "SELECT COALESCE(sum(CASE WHEN ?3 OR sequence<=?4 THEN 1 ELSE 0 END),0),COALESCE(max(sequence),0),COALESCE(max(CASE WHEN ?3 OR sequence<=?4 THEN sequence ELSE 0 END),0),COALESCE(sum(CASE WHEN NOT ?3 AND sequence>?4 THEN 1 ELSE 0 END),0) FROM public_history WHERE owner_user_id=?1 AND session_id=?2",
        params![owner,room,globally_finished,build_cursor as i64],|r|Ok((read_u64(r,0)?,read_u64(r,1)?,read_u64(r,2)?,read_u64(r,3)?))).map_err(public_error)?;
    let finished = pending == 0;
    let excluded_events: u64=connection.query_row("SELECT count(*) FROM history_events h WHERE h.session_id=?1
        AND h.kind IN ('user_prompt','provider_output','provider_tool','provider_error')
        AND NOT EXISTS(SELECT 1 FROM public_history_member m JOIN public_history p ON p.sequence=m.sequence
            WHERE m.event_ref=h.event_id AND p.owner_user_id=?2)",params![room,owner],|r|read_u64(r,0)).map_err(public_error)?;
    let retention_gap_events:u64=connection.query_row("SELECT COALESCE((SELECT deleted_events FROM public_history_retention WHERE session_id=?1),0)",[room],|r|read_u64(r,0)).map_err(public_error)?;
    let truncated_events:u64=connection.query_row("SELECT count(*) FROM public_history WHERE owner_user_id=?1 AND session_id=?2 AND json_extract(document_json,'$.truncated')=1",params![owner,room],|r|read_u64(r,0)).map_err(public_error)?;
    Ok(PublicHistoryCoverage {
        index_version: PUBLIC_HISTORY_VERSION,
        redaction_version: PUBLIC_HISTORY_VERSION,
        indexed_events,
        excluded_events,
        through_sequence,
        complete: excluded_events == 0
            && finished
            && retention_gap_events == 0
            && truncated_events == 0,
        rebuilding: !finished,
        rebuild_cursor,
        retention_gap_events,
        truncated_events,
    })
}

// MP-08 / MP-10 / MP-11: coalesce only events with identical admission
// metadata and stream identity. Raw events stay separate in operational history;
// public membership and the first retained reference remain stable.
fn same_public_stream(a: &HistoryEvent, b: &HistoryEvent) -> bool {
    a.kind == HistoryEventKind::ProviderOutput
        && b.kind == a.kind
        && a.public_history_owner_user_id == b.public_history_owner_user_id
        && a.session_id.is_some()
        && a.session_id == b.session_id
        && a.agent_id.is_some()
        && a.agent_id == b.agent_id
        && a.provider_run_id.is_some()
        && a.provider_run_id == b.provider_run_id
        && a.prompt_id.as_ref().or(a.turn_id.as_ref()).is_some()
        && a.prompt_id == b.prompt_id
        && a.turn_id == b.turn_id
        && history_event_merge_key(a).is_some()
        && a.metadata == b.metadata
        && a.provider == b.provider
        && a.model == b.model
        && a.content.is_some()
        && b.content.is_some()
        && a.content_ref == b.content_ref
        && a.role == b.role
        && a.workflow_id == b.workflow_id
        && a.workflow_run_id == b.workflow_run_id
        && a.workflow_node_id == b.workflow_node_id
}

pub(super) fn write_public_batch(
    connection: &Connection,
    events: &[&HistoryEvent],
    projector: &ProjectorSlot,
) -> Result<(), rusqlite::Error> {
    let mut groups: Vec<(HistoryEvent, Vec<&HistoryEvent>)> = Vec::new();
    let mut streams = BTreeMap::<_, Vec<usize>>::new();
    for event in events {
        let key = (event.kind == HistoryEventKind::ProviderOutput)
            .then(|| {
                Some((
                    event.public_history_owner_user_id.as_deref()?,
                    event.session_id.as_deref()?,
                    event.agent_id.as_deref()?,
                    event.provider_run_id.as_deref()?,
                    event.prompt_id.as_deref().or(event.turn_id.as_deref())?,
                    history_event_merge_key(event)?,
                ))
            })
            .flatten();
        let existing = key.and_then(|key| streams.get(&key)).and_then(|indices| {
            indices
                .iter()
                .copied()
                .find(|&index| same_public_stream(&groups[index].0, event))
        });
        if let Some(index) = existing {
            let (joined, members) = &mut groups[index];
            joined
                .content
                .as_mut()
                .expect("stream content")
                .push_str(event.content.as_deref().unwrap_or_default());
            joined.timestamp_ms = joined.timestamp_ms.min(event.timestamp_ms);
            members.push(event);
        } else {
            if let Some(key) = key {
                streams.entry(key).or_default().push(groups.len());
            }
            groups.push(((*event).clone(), vec![event]));
        }
    }
    for (mut event, members) in groups {
        if members.len() > 1 {
            event.metadata.insert(
                PUBLIC_HISTORY_BATCH_REFS.into(),
                serde_json::json!(members
                    .iter()
                    .map(|member| &member.event_id)
                    .collect::<Vec<_>>()),
            );
        }
        write_public_document(connection, &event, projector)?;
        for member in members.iter().skip(1) {
            connection.execute(
                "INSERT OR REPLACE INTO public_history_member(event_ref,sequence)
                 SELECT ?1,sequence FROM public_history_member WHERE event_ref=?2",
                params![member.event_id, event.event_id],
            )?;
        }
    }
    Ok(())
}

pub(super) fn write_public_document(
    connection: &Connection,
    event: &HistoryEvent,
    projector: &ProjectorSlot,
) -> Result<(), rusqlite::Error> {
    // Empty output carries no public text and must not erase a retained message.
    if event.kind == HistoryEventKind::ProviderOutput
        && event.content.as_deref().is_none_or(str::is_empty)
    {
        return Ok(());
    }
    connection.execute(
        "DELETE FROM public_history WHERE event_ref=?1
         OR sequence IN (SELECT sequence FROM public_history_member WHERE event_ref=?1)",
        [&event.event_id],
    )?;
    let existing = match (
        event.kind, event.public_history_owner_user_id.as_deref(),
        event.session_id.as_deref(), event.agent_id.as_deref(),
        history_event_merge_key(event), event.provider_run_id.as_deref(),
        event.prompt_id.as_deref().or(event.turn_id.as_deref()),
    ) {
        (HistoryEventKind::ProviderOutput, Some(owner), Some(room), Some(agent), Some(key), Some(run), Some(turn)) => connection
            .query_row(
                "SELECT p.sequence,p.document_json FROM public_history p
                 JOIN history_events h ON h.event_id=p.event_ref AND h.sequence=p.sequence
                 WHERE p.owner_user_id=?1 AND p.session_id=?2 AND p.agent_id=?3 AND p.kind='provider_output'
                   AND h.session_id=?2 AND h.agent_id=?3
                   AND h.provider_run_id=?4 AND h.merge_key=?5
                   AND COALESCE(h.prompt_id,json_extract(p.document_json,'$.turn_id'))=?6",
                params![owner, room, agent, run, key, turn],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            ).optional()?,
        _ => None,
    };
    let encode = |doc: &PublicHistoryDocument| {
        serde_json::to_string(doc).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
    };
    let sequence = if let Some((sequence, json)) = existing {
        let mut message: PublicHistoryDocument = serde_json::from_str(&json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;
        // Only a suffix can form a new secret with these deltas. The prefix
        // already passed this registry epoch; mutations drain/fence the writer.
        let bytes = projector.boundary_bytes(event);
        let mut boundary = message.text.len().saturating_sub(bytes);
        while !message.text.is_char_boundary(boundary) {
            boundary = boundary.saturating_sub(1);
        }
        let mut joined = event.clone();
        joined.content = Some(format!(
            "{}{}",
            &message.text[boundary..],
            event.content.as_deref().unwrap_or_default()
        ));
        let Some(projected) = projector.project(&joined) else {
            connection.execute("DELETE FROM public_history WHERE sequence=?1", [sequence])?;
            return Ok(());
        };
        // Scope fields are immutable authority, even under custom projectors.
        if projected.owner_user_id != message.owner_user_id
            || projected.session_id != message.session_id
            || projected.agent_id != message.agent_id
            || projected.kind != message.kind
        {
            return Ok(());
        }
        message.text.truncate(boundary);
        message.text.push_str(&projected.text);
        let end = message
            .text
            .char_indices()
            .nth(64 * 1024)
            .map(|(index, _)| index);
        message.truncated |= projected.truncated || end.is_some();
        if let Some(end) = end {
            message.text.truncate(end);
        }
        connection.execute(
            "UPDATE public_history SET text=?2,document_json=?3 WHERE sequence=?1",
            params![sequence, message.text, encode(&message)?],
        )?;
        sequence
    } else {
        let Some(doc) = projector.project(event) else {
            return Ok(());
        };
        connection.execute("INSERT INTO public_history(sequence,event_ref,owner_user_id,session_id,agent_id,kind,text,document_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![doc.sequence as i64,doc.event_ref,doc.owner_user_id,doc.session_id,doc.agent_id,history_event_kind_key(doc.kind),doc.text,encode(&doc)?])?;
        doc.sequence as i64
    };
    connection.execute(
        "INSERT OR REPLACE INTO public_history_member(event_ref,sequence) VALUES(?1,?2)",
        params![event.event_id, sequence],
    )?;
    Ok(())
}

pub(super) fn initialize_public_history(connection: &Connection) -> Result<(), DaemonError> {
    let index_existed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='public_history_fts')",
        [], |r| r.get(0),
    ).map_err(public_error)?;
    connection
        .execute_batch("PRAGMA secure_delete=ON; PRAGMA temp_store=MEMORY;")
        .map_err(public_error)?;
    let tx =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(public_error)?;
    tx.execute_batch(include_str!("public_history.sql"))
        .map_err(public_error)?;
    tx.commit().map_err(public_error)?;
    if !index_existed {
        // A recreated derived index resumes from the existing sanitized source.
        // Persist the reset before admitting writes/searches; raw history is never
        // a recovery source, and new appends follow the bounded rebuild cursor.
        connection
            .execute_batch("UPDATE public_history_build SET cursor=0,complete=0,epoch=epoch+1;")
            .map_err(public_error)?;
    }
    let version: u32 = connection
        .query_row("SELECT version FROM public_history_version", [], |r| {
            r.get(0)
        })
        .map_err(public_error)?;
    if version != PUBLIC_HISTORY_VERSION {
        // Never repopulate from old/raw provenance after a redaction/schema change.
        let tx = connection.unchecked_transaction().map_err(public_error)?;
        tx.execute_batch("DELETE FROM public_history; DELETE FROM public_history_member; UPDATE public_history_revision SET revision=revision+1; UPDATE public_history_build SET cursor=0,complete=0,epoch=epoch+1;").map_err(public_error)?;
        tx.execute(
            "UPDATE public_history_version SET version=?1",
            [PUBLIC_HISTORY_VERSION],
        )
        .map_err(public_error)?;
        tx.commit().map_err(public_error)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "public_history_tests.rs"]
mod tests;
