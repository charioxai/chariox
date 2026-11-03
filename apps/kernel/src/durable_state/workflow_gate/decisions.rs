use super::*;

fn state(db: &Connection, a: &Admission) -> Result<(u64, gate::State)> {
    let workflow:Option<(i64,String)>=db.query_row("SELECT revision,state_json FROM workflow_gate_state WHERE owner_id=?1 AND session_id=?2 AND workflow_id=?3 AND binding_id=''",params![a.owner,a.session_id,a.workflow_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql)?;
    let binding:Option<String>=db.query_row("SELECT state_json FROM workflow_gate_state WHERE owner_id=?1 AND session_id=?2 AND workflow_id=?3 AND binding_id=?4",params![a.owner,a.session_id,a.workflow_id,a.binding_id],|r|r.get(0)).optional().map_err(sql)?;
    let (revision, workflow) = workflow.unwrap_or((0, "{}".into()));
    Ok((
        revision as u64,
        gate::State {
            workflow: decode(&workflow)?,
            binding: decode(binding.as_deref().unwrap_or("{}"))?,
        },
    ))
}
pub(super) fn claim(tx: &Transaction<'_>, now: u64) -> Result<Option<Claim>> {
    retention::sweep(tx, now)?;
    for id in ids(tx,"SELECT entry_id FROM workflow_gate_entries WHERE disposition NOT IN ('queued','dropped','failed','expired','cancelled') AND deadline_ms<=?1 ORDER BY deadline_ms,sequence LIMIT 8",[now as i64])? {
        if let Some(mut stored)=load(tx,&id)? {if !stored.entry.disposition.is_terminal() {terminal(tx,&mut stored,Disposition::Expired,Some("gate_expired".into()))?;}}
    }

    let candidates=ids(tx,"SELECT entry_id FROM workflow_gate_entries e LEFT JOIN workflow_gate_lanes lane ON lane.owner_id=e.owner_id AND lane.session_id=e.session_id AND lane.workflow_id=e.workflow_id WHERE e.disposition IN ('ready','deferred') AND e.due_at_ms<=?1 AND NOT EXISTS(SELECT 1 FROM workflow_gate_entries active WHERE active.owner_id=e.owner_id AND active.session_id=e.session_id AND active.workflow_id=e.workflow_id AND active.disposition='evaluating') ORDER BY coalesce(lane.last_sequence,0),e.due_at_ms,e.sequence LIMIT 8",[now as i64])?;
    for id in candidates {
        let mut s = load(tx, &id)?.ok_or_else(|| error("gate candidate missing"))?;
        if now >= s.entry.deadline_ms {
            terminal(
                tx,
                &mut s,
                Disposition::Expired,
                Some("gate_expired".into()),
            )?;
            continue;
        }
        if !admission::current_stored(tx, &s)? {
            terminal(
                tx,
                &mut s,
                Disposition::Cancelled,
                Some("gate_definition_changed".into()),
            )?;
            continue;
        }
        if s.entry.evaluation_count >= gate::MAX_EVALUATIONS {
            terminal(
                tx,
                &mut s,
                Disposition::Failed,
                Some("gate_evaluation_limit".into()),
            )?;
            continue;
        }
        let retained = retention::retained_bytes(tx)?;
        if retained > 256 * 1024 * 1024 - 1024 * 1024
            || !retention::body_capacity(tx, &s.admission, 2 * 1024 * 1024)?
        {
            terminal(
                tx,
                &mut s,
                Disposition::Failed,
                Some("gate_history_limit".into()),
            )?;
            continue;
        }
        let (revision, state) = state(tx, &s.admission)?;
        let phase = if s.entry.evaluation_count == 0 {
            if s.batch.is_some() {
                gate::Phase::BatchFlush
            } else {
                gate::Phase::Trigger
            }
        } else {
            gate::Phase::Resume
        };
        let evaluation = gate::Evaluation {
            input: gate::Input {
                abi: gate::ABI.into(),
                entry_id: id.clone(),
                endpoint_id: s.admission.template.endpoint_id().into(),
                allowed_entry_nodes: s.admission.allowed_nodes.clone(),
                binding_id: s.admission.binding_id.clone(),
                phase,
                accepted_at_ms: s.entry.accepted_at_ms,
                evaluation_at_ms: now,
                deadline_ms: s.entry.deadline_ms,
                trigger: s.batch.is_none().then(|| s.admission.trigger.clone()),
                batch: s.batch.clone(),
            },
            configuration: gate::Configuration::new(
                s.entry.workflow_revision.to_string(),
                s.admission.definition.config.clone(),
            )
            .map_err(|e| error(e.to_string()))?,
            state,
        };
        if let Err(e) = evaluation.validate() {
            terminal(tx, &mut s, Disposition::Failed, Some(e.to_string()))?;
            continue;
        }
        let component: Vec<u8> = tx
            .query_row(
                "SELECT component FROM workflow_gate_modules WHERE owner_id=?1 AND digest=?2",
                params![s.admission.owner, s.admission.definition.module_sha256],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if s.batch.is_some() {
            tx.execute(
                "UPDATE workflow_gate_batches SET closed=1 WHERE entry_id=?1",
                [&id],
            )
            .map_err(sql)?;
        }
        let token = format!("{:032x}", rand::random::<u128>());
        tx.execute("INSERT INTO workflow_gate_lanes VALUES(?1,?2,?3,(SELECT coalesce(max(last_sequence),0)+1 FROM workflow_gate_lanes)) ON CONFLICT(owner_id,session_id,workflow_id) DO UPDATE SET last_sequence=excluded.last_sequence",params![s.admission.owner,s.admission.session_id,s.admission.workflow_id]).map_err(sql)?;
        s.claim = Some(token.clone());
        s.claimed_revision = revision;
        s.claimed_evaluation = Some(evaluation.clone());
        s.entry.evaluation_count += 1;
        s.entry.disposition = Disposition::Evaluating;
        save(tx, &s)?;
        return Ok(Some(Claim {
            stored: s,
            token,
            evaluation,
            component,
        }));
    }
    Ok(None)
}
pub(super) fn complete(
    tx: &Transaction<'_>,
    id: &str,
    token: &str,
    mut report: gate::Report,
    now: u64,
) -> Result<Entry> {
    let mut s = load(tx, id)?.ok_or_else(|| error("gate entry missing"))?;
    if s.claim.as_deref() != Some(token) || s.entry.disposition != Disposition::Evaluating {
        return Err(error("gate claim lost"));
    }
    let evaluation = s
        .claimed_evaluation
        .take()
        .ok_or_else(|| error("gate claim input missing"))?;
    if !admission::current_stored(tx, &s)? {
        terminal(
            tx,
            &mut s,
            Disposition::Cancelled,
            Some("gate_definition_changed".into()),
        )?;
        return Ok(s.entry);
    }
    if now >= s.entry.deadline_ms {
        report.outcome = Err(gate::Failure::GateInvalidTime);
    }
    let (revision, _) = state(tx, &s.admission)?;
    if revision != s.claimed_revision {
        return Err(error("gate state revision conflict"));
    }
    if report.profile != gate::PROFILE || report.fuel_consumed > gate::FUEL {
        report.outcome = Err(gate::Failure::GateInvalidOutput);
    }
    if let Ok(output) = &report.outcome {
        if let Err(e) = output.validate(&evaluation) {
            report.outcome = Err(e);
        }
    }
    // Batch admission validates its entire group before either state is written.
    if let Ok(output) = &report.outcome {
        if let gate::Decision::Batch {
            key,
            flush_at_ms,
            payload,
        } = &output.decision
        {
            tx.execute_batch("SAVEPOINT gate_batch").map_err(sql)?;
            if let Err(e) = batch(tx, &mut s, key, *flush_at_ms, payload) {
                tx.execute_batch("ROLLBACK TO gate_batch").map_err(sql)?;
                report.outcome = Err(e);
            }
            tx.execute_batch("RELEASE gate_batch").map_err(sql)?;
        }
    }
    let next_revision = revision + u64::from(report.outcome.is_ok());
    let record = DecisionRecord {
        evaluation_number: s.entry.evaluation_count,
        evaluation: Some(evaluation),
        report: Some(report.clone()),
        compacted: None,
        state_revision_before: revision,
        state_revision_after: next_revision,
    };
    tx.execute(
        "INSERT INTO workflow_gate_decisions VALUES(?1,?2,?3)",
        params![id, s.entry.evaluation_count, encode(&record)?],
    )
    .map_err(sql)?;
    match report.outcome {
        Err(failure) => {
            let expired = now >= s.entry.deadline_ms;
            terminal(
                tx,
                &mut s,
                if expired {
                    Disposition::Expired
                } else {
                    Disposition::Failed
                },
                Some(failure.to_string()),
            )?;
        }
        Ok(output) => {
            for (binding, json) in [
                ("", encode(&output.state.workflow)?),
                (
                    s.admission.binding_id.as_str(),
                    encode(&output.state.binding)?,
                ),
            ] {
                let count:i64=tx.query_row("SELECT count(*) FROM workflow_gate_state WHERE owner_id=?1 AND session_id=?2 AND workflow_id=?3 AND binding_id<>''",params![s.admission.owner,s.admission.session_id,s.admission.workflow_id],|r|r.get(0)).map_err(sql)?;
                let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM workflow_gate_state WHERE owner_id=?1 AND session_id=?2 AND workflow_id=?3 AND binding_id=?4)",params![s.admission.owner,s.admission.session_id,s.admission.workflow_id,binding],|r|r.get(0)).map_err(sql)?;
                if !binding.is_empty() && count >= 256 && !exists {
                    return Err(error("gate binding state capacity reached"));
                }
                tx.execute("INSERT INTO workflow_gate_state VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(owner_id,session_id,workflow_id,binding_id) DO UPDATE SET revision=excluded.revision,state_json=excluded.state_json",params![s.admission.owner,s.admission.session_id,s.admission.workflow_id,binding,next_revision as i64,json]).map_err(sql)?;
            }
            s.claim = None;
            match output.decision {
                gate::Decision::Run { targets } => {
                    s.routes = targets;
                    s.entry.disposition = Disposition::WaitingForQueue;
                    s.entry.next_due_at_ms = None;
                    save(tx, &s)?;
                }
                gate::Decision::Drop => terminal(tx, &mut s, Disposition::Dropped, None)?,
                gate::Decision::DeferUntil { at_ms } => {
                    s.entry.disposition = Disposition::Deferred;
                    s.entry.next_due_at_ms = Some(at_ms);
                    save(tx, &s)?;
                }
                gate::Decision::Batch { .. } => {
                    s.entry.disposition = Disposition::Batched;
                    s.entry.next_due_at_ms = None;
                    save(tx, &s)?;
                }
            }
        }
    }
    Ok(s.entry)
}

fn batch(
    tx: &Transaction<'_>,
    s: &mut Stored,
    key: &str,
    flush: u64,
    payload: &serde_json::Value,
) -> std::result::Result<(), gate::Failure> {
    let result = (|| -> Result<()> {
        let scope = gate::sha256(
            encode(&serde_json::json!([
                s.admission.owner,
                s.admission.session_id,
                s.admission.workflow_id,
                s.entry.workflow_revision,
                s.admission.binding_id,
                s.admission.template.endpoint_id(),
                s.admission.template.queue_id(),
                key
            ]))
            .map_err(|_| error("gate batch identity"))?
            .as_bytes(),
        );
        let existing: Option<(String, String, i64)> = tx
            .query_row(
                "SELECT group_id,entry_id,flush_at_ms FROM workflow_gate_batches WHERE scope_sha256=?1 AND closed=0",
                [&scope],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(sql)?;
        let group = existing
            .as_ref()
            .map(|row| row.0.clone())
            .unwrap_or_else(|| format!("batch_{:032x}", rand::random::<u128>()));
        let aggregate = if let Some((_, id, time)) = existing {
            if time != flush as i64 {
                return Err(error("gate batch cutoff conflict"));
            }
            id
        } else {
            let count:i64=tx.query_row("SELECT count(*) FROM workflow_gate_batches b JOIN workflow_gate_entries e ON e.entry_id=b.entry_id WHERE e.owner_id=?1 AND e.session_id=?2 AND e.workflow_id=?3 AND b.closed=0",params![s.admission.owner,s.admission.session_id,s.admission.workflow_id],|r|r.get(0)).map_err(sql)?;
            if count >= 64 {
                return Err(error("gate batch group limit"));
            }
            let mut a = s.admission.clone();
            a.dedupe_id = format!("batch:{group}");
            a.source_receipt = None;
            let entry = admission::accept_in(tx, a)?;
            let mut aggregate =
                load(tx, &entry.entry_id)?.ok_or_else(|| error("gate aggregate missing"))?;
            aggregate.batch = Some(gate::Batch {
                key: key.into(),
                flush_at_ms: flush,
                items: vec![],
            });
            aggregate.entry.disposition = Disposition::Deferred;
            aggregate.entry.next_due_at_ms = Some(flush);
            save(tx, &aggregate)?;
            tx.execute(
                "INSERT INTO workflow_gate_batches VALUES(?1,?2,?3,?4,0)",
                params![group, scope, entry.entry_id, flush as i64],
            )
            .map_err(sql)?;
            entry.entry_id
        };
        let (count,bytes):(i64,i64)=tx.query_row("SELECT count(*),coalesce(sum(length(payload_json)),0) FROM workflow_gate_members WHERE group_id=?1",[&group],|r|Ok((r.get(0)?,r.get(1)?))).map_err(sql)?;
        let item = gate::BatchItem {
            entry_id: s.entry.entry_id.clone(),
            payload: payload.clone(),
        };
        if count >= 256 || bytes + encode(&item)?.len() as i64 + count + 2 > 64 * 1024 {
            return Err(error("gate batch member limit"));
        }
        if !retention::body_capacity(tx, &s.admission, 2 * encode(&item)?.len() as i64)? {
            return Err(error("gate batch retained body limit"));
        }
        tx.execute(
            "INSERT INTO workflow_gate_members VALUES(?1,?2,?3)",
            params![group, s.entry.entry_id, encode(&item)?],
        )
        .map_err(sql)?;
        let mut a = load(tx, &aggregate)?.ok_or_else(|| error("gate aggregate missing"))?;
        a.entry.deadline_ms = a.entry.deadline_ms.min(s.entry.deadline_ms);
        if let Some(invocation) = a.admission.template.notification_invocation_mut() {
            if let Some(incoming) = s
                .admission
                .template
                .publication_invocation()
                .and_then(|i| i.input.get("ancestry"))
                .and_then(serde_json::Value::as_array)
            {
                let ancestry = invocation.input["ancestry"]
                    .as_array_mut()
                    .ok_or_else(|| error("notification ancestry missing"))?;
                for ancestor in incoming {
                    if !ancestry.contains(ancestor) {
                        ancestry.push(ancestor.clone());
                    }
                }
                if ancestry.len() > 256 {
                    return Err(error("gate batch ancestry limit"));
                }
            }
            if invocation.input.get("deadline_ms").is_some() {
                invocation.input["deadline_ms"] = serde_json::json!(a.entry.deadline_ms);
            }
        }
        a.batch
            .as_mut()
            .ok_or_else(|| error("gate aggregate data missing"))?
            .items
            .push(item);
        save(tx, &a)?;
        Ok(())
    })();
    result.map_err(|_| gate::Failure::GateLimit)
}

pub(super) fn release(
    tx: &Transaction<'_>,
    id: &str,
    before: &RuntimeSession,
    after: &RuntimeSession,
) -> Result<()> {
    let mut s = load(tx, id)?.ok_or_else(|| error("gate entry missing"))?;
    if s.entry.disposition != Disposition::WaitingForQueue {
        return Err(error("gate dispatch intent already consumed"));
    }
    if crate::session::unix_epoch_ms() >= s.entry.deadline_ms {
        terminal(
            tx,
            &mut s,
            Disposition::Expired,
            Some("gate_expired".into()),
        )?;
        return Ok(());
    }
    if !admission::current_stored(tx, &s)? {
        terminal(
            tx,
            &mut s,
            Disposition::Cancelled,
            Some("gate_definition_changed".into()),
        )?;
        return Ok(());
    }
    let expected = super::super::workflow_runtime::encode_workflow_session(before)?;
    if !super::super::workflow_runtime::hot_state_matches(tx, before.host_daemon_id(), &expected)
        .map_err(sql)?
    {
        return Err(error("gate queue snapshot conflict"));
    }
    let queued = after
        .workflow_queued_prompts()
        .iter()
        .find(|q| q.id() == s.admission.template.id())
        .ok_or_else(|| error("gate queue handoff missing"))?;
    if queued.gate_entry_id() != Some(id) {
        return Err(error("gate queue proof missing"));
    }
    let encoded = super::super::workflow_runtime::encode_workflow_session(after)?;
    super::super::app_event_delivery::write_queue_state_in(
        tx,
        after,
        &encoded,
        "workflow_gate_queued",
        id,
        super::super::workflow_invocation_provenance::WorkflowUserInvocation {
            queue_item_id: queued.id().into(),
            user_id: s.admission.owner.clone(),
        },
    )?;
    s.entry.queued_prompt_id = Some(queued.id().into());
    terminal(tx, &mut s, Disposition::Queued, None)?;
    Ok(())
}
