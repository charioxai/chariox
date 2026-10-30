// MP-08 / MP-10 / MP-11: supplementary orchestration checks; acceptance uses real providers.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

let serial=0;
const moduleUrl=source=>'data:text/javascript;base64,'+Buffer.from(source).toString('base64');
async function runPairs(filter){
 const key='__contextPairsTest'+(++serial),state={web:[],native:0};
 state.select=async(provider,model)=>{if(state.provider!==provider)state.nativeId='native-'+(++state.native);state.provider=provider;state.model=model;};
 state.turn=async()=>{if(state.provider==='opencode'&&++state.openCodeTurns>3)throw Error('OpenCode turn budget exceeded');};
 state.workload=async(provider,model)=>{state.openCodeTurns=0;await state.select(provider,model);await state.turn();return {};};
 state.record=async(name,result)=>{if(name==='result')state.result=structuredClone(result);};
 globalThis[key]=state;
 const common=moduleUrl(`const s=globalThis[${JSON.stringify(key)}];
  export const r={binding:async()=>({provider:s.provider,model:s.model,accountProfile:s.provider+'-account',nativeId:s.nativeId}),turn:s.turn,record:s.record,capture:async()=>{}};
  export const init=async()=>({}),enrollClaude=async()=>{},select=s.select,workload=s.workload,probe=s.turn,finish=async()=>{};
 `);
 const viewers=moduleUrl(`const s=globalThis[${JSON.stringify(key)}];
  export async function webClients(){return [1,2].map(dpr=>({dpr,select:s.select,close:async()=>{},capture:async()=>{},probe:async(facts,ten)=>{await s.turn();s.web.push({provider:s.provider,dpr,ten});return {score:ten?10:9,total:ten?10:9};}}));}
 `);
 let source=await readFile(new URL('./claude-pairs.mjs',import.meta.url),'utf8');
 source=source.replace("from './common.mjs'",'from '+JSON.stringify(common))
  .replace("from './web-live.mjs'",'from '+JSON.stringify(viewers))
  .replace("from './context-switch-validation-brief.mjs'",'from '+JSON.stringify(new URL('./context-switch-validation-brief.mjs',import.meta.url).href));
 const oldFilter=process.env.CTXVAL_PAIRS,oldExit=process.exitCode;
 process.env.CTXVAL_PAIRS=filter;
 try{await import(moduleUrl(source+'\n// fixture '+serial));state.exitCode=process.exitCode;return state;}
 finally{if(oldFilter===undefined)delete process.env.CTXVAL_PAIRS;else process.env.CTXVAL_PAIRS=oldFilter;process.exitCode=oldExit;delete globalThis[key];}
}

test('MP-08 / MP-10 / MP-11 Codex model changes check both DPRs on both legs',async()=>{
 const {result}=await runPairs('codex-codex');
 assert.equal(result.status,'PASS');assert.equal(result.cells.length,1);
 const [row]=result.cells;
 assert.equal(row.source.model,'gpt-6-luna');
 assert.deepEqual(row.legs.map(leg=>leg.target.model),['gpt-6.1-sol','gpt-6-luna']);
 for(const [index,leg]of row.legs.entries()){
  assert.deepEqual(leg.scores.map(score=>score.dpr),[1,2]);
  assert(leg.scores.every(score=>score.score===(index===0?9:10)));
 }
});

test('MP-08 / MP-10 / MP-11 both provider directions check both DPRs per leg',async()=>{
 const {result}=await runPairs('codex-claude-p,claude-p-codex');
 assert.equal(result.status,'PASS');assert.equal(result.cells.length,2);
 for(const row of result.cells)for(const leg of row.legs)assert.deepEqual(leg.scores.map(score=>score.dpr),[1,2]);
});

test('MP-08 / MP-10 / MP-11 extra DPR probes preserve the three-turn OpenCode budget',async()=>{
 const {result,web}=await runPairs('claude-p-opencode,opencode-claude-p');
 assert.equal(result.status,'PASS',JSON.stringify(result.cells.map(row=>row.firstSeam)));
 assert.equal(result.cells.length,2);
 assert.equal(web.filter(probe=>probe.provider==='opencode').length,2);
});

test('MP-08 / MP-10 / MP-11 an unmatched pair selection cannot pass an empty run',async()=>{
 const {result,exitCode,web}=await runPairs('not-an-offered-pair');
 assert.equal(result.status,'FAIL');assert.equal(exitCode,1);
 assert.match(result.firstSeam,/No context-switch pairs matched/);
 assert.deepEqual(result.cells,[]);assert.deepEqual(web,[]);
});
