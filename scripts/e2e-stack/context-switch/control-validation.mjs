// MP-08 / MP-10 / MP-11: acceptance oracles shared by real runs and negative regressions.
const canonicalModel=value=>String(value??'').replace(/^claude(?:-p|-headless)?\//,'');
export function validateCompactionOutage({baseline,events,elapsedMs,sourceRunId,sourceModel,targetModel}){
 if(events.length!==baseline.length+1||!/failed to compact the session before/.test(events.at(-1).message))throw Error('Scoped source outage did not log one precompaction failure');
 const event=events.at(-1),fields=event.fields??{};
 if(event.errorClass!=='timeout')throw Error('Source compaction failure was not a timeout');
 if(!Number.isFinite(fields.elapsed_ms)||fields.elapsed_ms<299000||fields.elapsed_ms>310000)throw Error('Source compaction deadline elapsed evidence must be approximately300s');
 if(!sourceRunId||fields.provider_run_id!==sourceRunId||canonicalModel(fields.source_model)!==canonicalModel(sourceModel)||canonicalModel(fields.target_model)!==canonicalModel(targetModel))throw Error('Compaction timeout does not match the selected source operation');
 if(!Number.isFinite(elapsedMs)||elapsedMs<fields.elapsed_ms||elapsedMs>320000)throw Error('Profile did not settle within300s plus normal relay response budget');
 return event;
}
export function validateAccountNativeSwitch({before,target,after,primary,secondary}){
 if(target.accountProfile!==secondary||after.accountProfile!==primary)throw Error('Independent account did not roundtrip');
 if(!before.nativeId||!target.nativeId||!after.nativeId||new Set([before.nativeId,target.nativeId,after.nativeId]).size!==3)throw Error('Independent account transitions must create fresh native sessions');
}

// MP-08 / MP-10 / MP-11: one dispatch, one new timeout, and the kernel's own clock.
export function validateBriefDeadline({baseline,events,elapsedMs,submittedAtMs,sessionId,agentId,targetRunId}){
 if(!sessionId||!agentId||!targetRunId||!Number.isFinite(submittedAtMs))throw Error('Brief dispatch identity is missing');
 if(events.length<baseline.length||baseline.some((event,index)=>JSON.stringify(event)!==JSON.stringify(events[index])))throw Error('Brief log baseline changed');
 const matching=events.slice(baseline.length).filter(event=>event.message==="failed to update the agent's handoff brief; the handoff keeps the stored brief"&&event.errorClass==='timeout'&&Number.isFinite(event.atMs)&&event.atMs>=submittedAtMs&&event.fields?.session_id===sessionId&&event.fields?.agent_id===agentId&&event.fields?.target_provider_run_id===targetRunId);
 if(matching.length!==1)throw Error('Expected one new matching brief timeout for this dispatch');
 const event=matching[0],elapsed=event.fields.elapsed_ms;
 if(!Number.isFinite(elapsed)||elapsed<119000||elapsed>125000)throw Error('Brief deadline elapsed evidence must be approximately120s');
 if(!Number.isFinite(elapsedMs)||elapsedMs<elapsed||elapsedMs>150000)throw Error('Brief did not settle within120s plus the bounded provider recovery/response budget');
 return event;
}
