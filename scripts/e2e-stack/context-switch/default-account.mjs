// MP-08 / MP-10 / MP-11: default selection must retain the real Claude session.
import {r,init,finish,enrollClaude,workload,probe} from './common.mjs';
import {webClients} from './web-live.mjs';
import {sleep} from './runtime.mjs';
let web=[],remote;const result={MP:['MP-08','MP-10','MP-11'],status:'FAIL',started:new Date().toISOString()};
try{
 ({remote}=await init());await enrollClaude();
 for(const provider of ['codex','opencode'])await r.client.send({TeardownProviderProcesses:{provider,force:false}});
 const facts=await workload(process.env.CTXVAL_CLAUDE_PROVIDER??'claude-p','sonnet[1m]',Number(process.env.CTXVAL_SOURCE_TURNS??15));let before=await r.binding();if(process.env.CTXVAL_REQUIRE_LARGE_DEFAULT==='1'){const files=['apps/kernel/src/runtime/state/local_prompt_dispatch_runtime.rs','apps/kernel/src/app/provider_output_claude_native.rs','apps/kernel/src/runtime/state/relay_peer_runtime_state.rs','apps/kernel/src/runtime/state/agent_config_runtime_state.rs'];let extra=0;while((before.runs.find(x=>x.provider===before.provider)?.contextTokens??0)<=160000&&extra<12){const file=files[extra%files.length];await r.turn('Continue the real read-only audit. Read '+file+' and its directly relevant tests. Review another source-backed cancellation, prompt ownership or native-session invariant not covered earlier, quoting the needed non-secret source excerpts. Preserve all nine audit records unchanged; do not echo their JSON or proposal. Do not edit files, run builds or read credentials/runtime state.','CTXFIX_DEFAULT_LARGE_'+extra,900000);before=await r.binding();extra++}if((before.runs.find(x=>x.provider===before.provider)?.contextTokens??0)<=160000)throw Error('Actual final API input did not cross the required large-default fixture160k gate');result.additionalSourceAuditTurns=extra;result.measuredSourceContextTokens=before.runs.find(x=>x.provider===before.provider)?.contextTokens;result.windowEvidenceLimit='Model selection source [1m] and target haiku; official /context window proof remains a separate CX04 gate'}result.before=before;
 if(!before.nativeId)throw Error('Default-created source has no observed native conversation');result.selectorEvidence='Real /session new created omitted/default account; later public projection may show resolved profile ID; no claim that the literal selector remained stored';
 web=await webClients(r,remote);for(const c of web)await c.capture('before-default-model-change');
 await r.command(`agent model ${before.agentId} claude-haiku-5-5`);
 let committed=false;for(let i=0;i<480;i++){const s=await r.tui.send('snapshot'),a=s.session.agents.find(a=>a.id===before.agentId);if(a.model==='claude-haiku-5-5'||a.model==='claude-p/claude-haiku-5-5'){committed=true;break}await sleep(250)}if(!committed)throw Error('Real TUI model control did not commit');
 await probe(facts);const after=await r.binding();result.after=after;result.nativeIdPreserved=before.nativeId===after.nativeId;
 for(const c of web){await c.probe(facts);await c.capture('after-default-model-change');}
 if(!result.nativeIdPreserved)throw Error('Same-account default downshift discarded its native Claude session');
 result.status='PASS';
}catch(e){result.firstSeam=String(e?.message??e);await r.capture('default-account-failed').catch(()=>{});await Promise.allSettled(web.map(c=>c.capture('default-account-failed')));}
finally{result.finished=new Date().toISOString();await r.record('result',result);for(const c of web)await c.close().catch(()=>{});await finish();}
if(result.status!=='PASS')process.exitCode=1;
