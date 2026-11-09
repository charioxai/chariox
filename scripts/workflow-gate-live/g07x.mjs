// G-07 two owners (two kernels, two homes): another owner's gate entries, journal and decisions are invisible and
// not cancellable; the exported workflow carries the gate config (module digest + config digest) but no entries.
import {Rig,boot,submit,sleep,recorder,isTerminal,jq,Checks,R} from './lib.mjs';
const rec=recorder('g07x');const C=new Checks(rec);
const a=new Rig('g07xa');await a.start();await a.session();await boot(a);const A=a.alias;
const done=(await Promise.all([submit(a,'run'),submit(a,'drop')])).map(e=>e.entry_id);
for(const id of done)await a.waitFor(A,id,isTerminal,30000);
const held=await submit(a,'defer:300');await a.waitFor(A,held.entry_id,x=>x.disposition==='deferred',30000);
const exp=(await a.send({ExportWorkflowCodeSource:{session_id:a.sessionId,target:{kind:'workflow',workflow_ref:A},format:'inline',agent_mode:'portable_generated'}})).WorkflowCodeSourceExported.export;
const cfgCarried=/gate_v1/.test(exp.source)&&exp.source.includes((await a.entry(A,done[0])).module_sha256)&&exp.source.includes((await a.entry(A,done[0])).config_sha256);
const leaked=done.concat(held.entry_id).some(id=>exp.source.includes(id));
C.ok('X1','export carries the gate config (abi, module digest, config digest) and no entry id',cfgCarried&&!leaked,{cfgCarried,leaked,bytes:exp.source.length});
const b=new Rig('g07xb');await b.start();await b.session();await b.newWorkflow('gate-upload');await b.put('gate-upload',R+'/matrix-gate.wasm');await b.apply(exp.source);b.alias=A;
const bl=await b.allEntries(A);
C.ok('X2','other owner: entries list empty, journal/decision tables empty, state revision 0',bl.length===0&&jq(b,'select count(*) from workflow_gate_entries')[0][0]===0&&jq(b,'select count(*) from workflow_gate_decisions')[0][0]===0&&(await b.state(A)).revision===0,{listed:bl.length});
let probe,cancelErr;try{probe=JSON.stringify(await b.entry(A,held.entry_id)).slice(0,160)}catch(e){probe='ERR '+String(e).slice(0,160)}
try{await b.cancel(A,held.entry_id,'foreign cancel');cancelErr=null}catch(e){cancelErr=String(e).slice(0,160)}
C.ok('X3','other owner cannot read or cancel the first owner\'s deferred entry',/ERR|not found/i.test(probe)&&cancelErr&&/not found/.test(cancelErr),{probe,cancelErr});
const still=await a.entry(A,held.entry_id);const ev=await a.allEntries(A);
C.ok('X4','first owner\'s deferred entry untouched by the foreign cancel; its own cancel works',still.disposition==='deferred'&&(await a.cancel(A,held.entry_id,'own cancel'),(await a.entry(A,held.entry_id)).disposition==='cancelled'),{before:still.disposition,entries:ev.length});
const nb=await submit(b,'run');const fb=await b.waitFor(A,nb.entry_id,isTerminal,30000);
C.ok('X5','other owner evaluates under its own state: new entry queued, first owner entries still 4 vs other owner 1',fb.disposition==='queued'&&(await a.allEntries(A)).length===3&&(await b.allEntries(A)).length===1,{disp:fb.disposition});
console.log(JSON.stringify({failed:C.failed.length,total:C.rows.length}));
await b.stop();await a.stop();
