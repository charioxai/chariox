use super::*;

pub(super) fn current(db: &Connection, a: &Admission) -> Result<bool> {
    let json:Option<String>=db.query_row("SELECT payload_json FROM durable_workflow_hot_entities WHERE owner_id=?1 AND session_id=?2 AND entity_kind='workflow' AND entity_id=?3",params![a.kernel_id,a.template_session(),a.workflow_id],|r|r.get(0)).optional().map_err(sql)?;
    let Some(json) = json else { return Ok(false) };
    let workflow: crate::session::WorkflowDefinition = decode(&json)?;
    let queue_exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM durable_workflow_hot_entities WHERE owner_id=?1 AND session_id=?2 AND entity_kind='queue' AND entity_id=?3 AND json_extract(payload_json,'$.workflow_id')=?4)",params![a.kernel_id,a.session_id,a.template.queue_id(),a.workflow_id],|r|r.get(0)).map_err(sql)?;
    Ok(queue_exists
        && workflow.revision() == a.workflow_revision
        && workflow.gate() == Some(&a.definition)
        && workflow
            .endpoint(a.template.endpoint_id())
            .is_some_and(|e| e.owner_user_id() == a.owner))
}
impl Admission {
    pub(super) fn template_session(&self) -> &str {
        &self.session_id
    }
}
/// Binds a dedupe id to its occurrence: a retry with other content conflicts.
pub(super) fn content_digest(a: &Admission) -> Result<String> {
    let content = serde_json::json!([
        a.owner,
        a.session_id,
        a.workflow_id,
        a.workflow_revision,
        a.binding_id,
        a.dedupe_id,
        a.caller_sha256,
        a.definition,
        a.trigger,
        a.allowed_nodes,
        a.template.endpoint_id(),
        a.template.queue_id(),
        a.template.created_by_agent_id(),
        a.template.publication_invocation().map(|i| (
            &i.publication_id,
            &i.hook_id,
            &i.transport,
            &i.caller
        )),
        a.source_receipt.as_ref().map(|_| a.deadline_ms)
    ]);
    Ok(gate::sha256(
        &gate::canonical_json(&content, gate::INPUT_BYTES).map_err(|e| error(e.to_string()))?,
    ))
}
/// The caller identity a gate sees: the invoking user and the creating agent.
pub(crate) fn caller_sha256(user: &str, creator: Option<&str>) -> Result<String> {
    Ok(gate::sha256(
        encode(&serde_json::json!([user, creator]))?.as_bytes(),
    ))
}
pub(crate) fn accept_in(tx: &Transaction<'_>, a: Admission) -> Result<Entry> {
    retention::sweep(tx, crate::session::unix_epoch_ms())?;
    a.definition.validate().map_err(|e| error(e.to_string()))?;
    let digest = content_digest(&a)?;
    if let Some((id,existing))=tx.query_row("SELECT entry_id,content_digest FROM workflow_gate_entries WHERE owner_id=?1 AND session_id=?2 AND workflow_id=?3 AND binding_id=?4 AND dedupe_id=?5",params![a.owner,a.session_id,a.workflow_id,a.binding_id,a.dedupe_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional().map_err(sql)? {
        // Queue IDs/timestamps are authority-allocated on retries. The immutable
        // source identity binds content instead of that freshly prepared template.
        let old=entry(tx,&id)?.ok_or_else(||error("gate dedupe entry missing"))?;
        if existing!=digest {return Err(error("gate occurrence content conflict"));}
        return Ok(old);
    }
    if !current(tx, &a)? {
        return Err(error("gate workflow revision/owner changed"));
    }
    let module: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM workflow_gate_modules WHERE owner_id=?1 AND digest=?2)",
            params![a.owner, a.definition.module_sha256],
            |r| r.get(0),
        )
        .map_err(sql)?;
    if !module {
        return Err(error("gate module is not installed for this owner"));
    }
    let (bindings,existing):(i64,bool)=tx.query_row("SELECT count(*),coalesce(max(binding_id=?4),0) FROM workflow_gate_state WHERE owner_id=?1 AND session_id=?2 AND workflow_id=?3 AND binding_id<>''",params![a.owner,a.session_id,a.workflow_id,a.binding_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(sql)?;
    if bindings >= 256 && !existing {
        return Err(error("gate binding state capacity reached"));
    }
    let now = crate::session::unix_epoch_ms();
    let evaluation = gate::Evaluation {
        input: gate::Input {
            abi: gate::ABI.into(),
            entry_id: "admission".into(),
            endpoint_id: a.template.endpoint_id().into(),
            allowed_entry_nodes: a.allowed_nodes.clone(),
            binding_id: a.binding_id.clone(),
            phase: gate::Phase::Trigger,
            accepted_at_ms: now,
            evaluation_at_ms: now,
            deadline_ms: a.deadline_ms,
            trigger: Some(a.trigger.clone()),
            batch: None,
        },
        configuration: gate::Configuration::new(
            a.workflow_revision.to_string(),
            a.definition.config.clone(),
        )
        .map_err(|e| error(e.to_string()))?,
        state: gate::State::default(),
    };
    evaluation.validate().map_err(|e| error(e.to_string()))?;
    let (pending, total): (i64, i64) = tx
        .query_row(
            "SELECT coalesce(sum(terminal_at_ms IS NULL),0),count(*) FROM workflow_gate_entries",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(sql)?;
    let local:i64=tx.query_row("SELECT count(*) FROM workflow_gate_entries WHERE owner_id=?1 AND session_id=?2 AND workflow_id=?3 AND disposition NOT IN ('queued','dropped','failed','expired','cancelled')",params![a.owner,a.session_id,a.workflow_id],|r|r.get(0)).map_err(sql)?;
    if pending >= 4096
        || local >= 1024
        || total - pending >= 65_536
        || retention::decision_reservations(tx)? + 32 > 262_144
        || !retention::body_capacity(tx, &a, encode(&a)?.len() as i64 + 4096)?
    {
        return Err(error(
            "gate admission backpressure; retained entry/history capacity reached",
        ));
    }
    let entry = Entry {
        entry_id: format!("gate_{:032x}", rand::random::<u128>()),
        session_id: a.session_id.clone(),
        workflow_id: a.workflow_id.clone(),
        workflow_revision: a.workflow_revision,
        module_sha256: a.definition.module_sha256.clone(),
        config_sha256: a.definition.config_sha256.clone(),
        binding_id: a.binding_id.clone(),
        disposition: Disposition::Ready,
        accepted_at_ms: now,
        deadline_ms: a.deadline_ms,
        next_due_at_ms: Some(now),
        evaluation_count: 0,
        queued_prompt_id: None,
        failure: None,
        decisions: vec![],
        replay_compacted_at_ms: None,
    };
    let source_authority = source_authority(tx, &a)?;
    if (a.source_receipt.is_some() || a.trigger.kind == gate::TriggerKind::Schedule)
        && source_authority.is_none()
    {
        return Err(error("gate source authority retired"));
    }
    let stored = Stored {
        entry: entry.clone(),
        source_authority,
        admission: a,
        claim: None,
        claimed_evaluation: None,
        claimed_revision: 0,
        batch: None,
        routes: vec![],
    };
    tx.execute("INSERT INTO workflow_gate_entries(entry_id,owner_id,session_id,workflow_id,binding_id,dedupe_id,content_digest,disposition,due_at_ms,deadline_ms,body_json) VALUES(?1,?2,?3,?4,?5,?6,?7,'ready',?8,?9,?10)",params![entry.entry_id,stored.admission.owner,entry.session_id,entry.workflow_id,entry.binding_id,stored.admission.dedupe_id,digest,now as i64,entry.deadline_ms as i64,encode(&stored)?]).map_err(sql)?;
    Ok(entry)
}

impl Admission {
    #[allow(
        clippy::too_many_arguments,
        reason = "Capture independent trusted source facts at one admission boundary before guest execution"
    )]
    pub(crate) fn from_prompt(
        session: &RuntimeSession,
        workflow: &crate::session::WorkflowDefinition,
        template: WorkflowQueuedPrompt,
        binding_id: String,
        kind: gate::TriggerKind,
        occurrence_id: String,
        deadline_ms: u64,
        data: Option<gate::JsonObject>,
        data_schema: Option<gate::DataSchema>,
        source_receipt: Option<String>,
    ) -> Result<Self> {
        let endpoint = workflow
            .endpoint(template.endpoint_id())
            .ok_or_else(|| error("gate endpoint missing"))?;
        Ok(Self {
            kernel_id: session.host_daemon_id().into(),
            session_id: session.id().into(),
            owner: endpoint.owner_user_id().into(),
            workflow_id: workflow.id().into(),
            workflow_revision: workflow.revision(),
            definition: workflow
                .gate()
                .cloned()
                .ok_or_else(|| error("gate not configured"))?,
            binding_id,
            dedupe_id: occurrence_id.clone(),
            caller_sha256: caller_sha256(endpoint.owner_user_id(), template.created_by_agent_id())?,
            deadline_ms,
            trigger: gate::Trigger {
                kind,
                occurrence_id,
                occurred_at_ms: None,
                data,
                data_schema,
                prompt: template.prompt().map(str::to_string),
                artifacts: template
                    .publication_invocation()
                    .map(|i| i.artifacts.clone())
                    .unwrap_or_default(),
            },
            allowed_nodes: endpoint
                .entry_node_ids()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            template,
            source_receipt,
            notification: None,
        })
    }
}

pub(super) fn source_authority(db: &Transaction<'_>, a: &Admission) -> Result<Option<String>> {
    if a.trigger.kind == gate::TriggerKind::Schedule {
        let schedule_id = a
            .trigger
            .data
            .as_ref()
            .and_then(|d| d.get("schedule_id"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| error("schedule identity missing"))?;
        let body:Option<String>=db.query_row("SELECT payload_json FROM durable_workflow_hot_entities WHERE owner_id=?1 AND session_id=?2 AND entity_kind='schedule' AND entity_id=?3",params![a.kernel_id,a.session_id,schedule_id],|r|r.get(0)).optional().map_err(sql)?;
        return body
            .map(|body| {
                let schedule = decode::<crate::session::WorkflowScheduleDefinition>(&body)?;
                // Consuming the last wakeup stops future ticks, not this accepted
                // entry. An owner's explicit disable has a distinct status.
                let accepted_last_tick = schedule
                    .max_wakeups()
                    .is_some_and(|max| schedule.wakeups_executed() >= max)
                    && schedule
                        .last_status()
                        .is_some_and(|s| s.starts_with("gate_entry:"));
                if !schedule.enabled() && !accepted_last_tick {
                    return Ok(None);
                }
                super::schedule::revision(&schedule).map(Some)
            })
            .transpose()
            .map(Option::flatten);
    }
    let Some(receipt) = &a.source_receipt else {
        return Ok(None);
    };
    let row:Option<(String,String,String)>=db.query_row("SELECT r.source_kind,r.installation_id,json_array(a.revision,a.status,a.event_name,a.event_version,a.schema_digest,a.session_id,a.publication_id,a.endpoint_id,a.queue_id,a.notification_json) FROM app_outbox r JOIN app_automations a ON a.owner_id=r.owner_id AND a.installation_id=r.installation_id AND a.automation_id=r.automation_id WHERE r.receipt_id=?1 AND r.owner_id=?2 AND a.status='active' AND a.revision=r.automation_revision",params![receipt,a.owner],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(sql)?;
    let Some((kind, installation, automation)) = row else {
        return Ok(None);
    };
    if kind == "workflow_completion" {
        let subscription:String=db.query_row("SELECT a.notification_json FROM app_outbox r JOIN app_automations a ON a.owner_id=r.owner_id AND a.installation_id=r.installation_id AND a.automation_id=r.automation_id WHERE r.receipt_id=?1",[receipt],|r|r.get(0)).map_err(sql)?;
        let sub: crate::local::WorkflowNotificationSubscription = decode(&subscription)?;
        // This performs the same ancestry/grant attachment check as the original
        // queue handoff. A saved gate result cannot outlive its source grant.
        super::super::workflow_notifications::require_attached(db, &sub, &installation)?;
        return Ok(Some(automation));
    }
    let installation:Option<String>=db.query_row("SELECT json_array(i.generation,i.active_json,t.package_digest,t.publisher_id,t.key_id,t.trust_revision,k.revision,hex(k.public_key),k.enrolled,u.authority_json) FROM app_installations i JOIN app_installation_stage_trust t ON t.installation_id=i.installation_id AND t.generation=i.generation LEFT JOIN app_publisher_keys k ON k.owner_id=i.owner_id AND k.publisher_id=t.publisher_id AND k.key_id=t.key_id LEFT JOIN app_installation_authority u ON u.installation_id=i.installation_id AND u.generation=i.generation WHERE i.installation_id=?1 AND i.owner_id=?2 AND i.admission_paused=0 AND i.active_json IS NOT NULL AND (k.enrolled IS NULL OR k.enrolled=1)",params![installation,a.owner],|r|r.get(0)).optional().map_err(sql)?;
    Ok(Some(encode(&(
        automation,
        match installation {
            Some(installation) => installation,
            None => return Ok(None),
        },
    ))?))
}
pub(super) fn current_stored(tx: &Transaction<'_>, s: &Stored) -> Result<bool> {
    if !current(tx, &s.admission)? || source_authority(tx, &s.admission)? != s.source_authority {
        return Ok(false);
    }
    if s.batch.is_some() {
        for id in ids(tx,"SELECT entry_id FROM workflow_gate_members WHERE group_id IN (SELECT group_id FROM workflow_gate_batches WHERE entry_id=?1)",[&s.entry.entry_id])? {
            let member=load(tx,&id)?.ok_or_else(||error("gate batch member missing"))?;
            if member.entry.disposition != Disposition::Batched || !current(tx,&member.admission)? || source_authority(tx,&member.admission)?!=member.source_authority {return Ok(false);}
        }
    }
    Ok(true)
}
