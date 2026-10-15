// MP-08 / MP-10 / MP-11: real TUI rejects an unsigned account without losing the typed draft/file.
import {r,init,finish} from './common.mjs';
import {writeFile} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {sleep} from './runtime.mjs';
const result={MP:['MP-08','MP-10','MP-11'],status:'FAIL',started:new Date().toISOString()};
try{
 await init();await r.launch('codex','gpt-6-luna');const state=await r.tui.send('snapshot'),agentId=state.session.focusedAgentId,primary=r.profiles.codex,label='ctx13-unsigned-'+Date.now();
 await r.command('provider accounts add codex '+label);
 const listed=await r.client.send({ListProviderAccountProfiles:{provider:'codex'}}),profile=listed.ProviderAccountProfilesListed.profiles.find(p=>p.label===label);if(!profile)throw Error('Unsigned profile registration missing');
 await r.command('agent account '+agentId+' '+label);let selected=false;
 for(let n=0;n<120;n++){const s=await r.tui.send('snapshot');if(s.session.agents.find(a=>a.id===agentId)?.accountProfile===profile.profile_id){selected=true;break;}await sleep(250);}
 if(!selected)throw Error('Unsigned account selection did not commit');
 const note=r.root+'/user-note.txt';await writeFile(note,'The audit note says preserve the user draft.\n');await r.command('attach '+note);
 const prompt='Read the attached user note and report its one audit rule. End with CX13_DRAFT_RETRY.';
 await r.tui.type(' '+prompt);await r.capture('typed-draft-with-file');await r.tui.type('\r');await sleep(1500);const rejected=await r.capture('account-block-restored-draft');
 if(rejected.footer?.tone!=='error'||!/sign.in|signed.in|not.*authenticated|needs.*login/i.test(rejected.footer.message??''))throw Error('Expected visible account-readiness rejection');
 const ansi=r.out+'/account-block-restored-draft.ansi';
 const screen=execFileSync(process.env.CTXVAL_PYTHON??'python3',['-c',"import pyte,sys; s=type('Screen',(pyte.Screen,),{'report_device_status':lambda self,mode,**kw:None})(160,48); pyte.Stream(s).feed(open(sys.argv[1]).read()); print('\\n'.join(s.display))",ansi],{encoding:'utf8'});
 await r.record('restored-editor-proof',{promptVisible:screen.includes('Read the attached user note'),attachmentVisible:screen.includes('[file 1]')||screen.includes('user-note.txt'),noAcceptedUserRows:!(rejected.agentPanes[agentId]??[]).some(x=>x.role==='user'&&x.text?.includes('CX13_DRAFT_RETRY')),footer:rejected.footer});
 if(!screen.includes('Read the attached user note')||(!screen.includes('[file 1]')&&!screen.includes('user-note.txt')))throw Error('Real cleared editor or attachment was not restored');
 // Use the visible account-readiness popup so commands never erase the restored draft/file.
 await r.tui.type('\x1b[B\r');let switched=false;
 for(let n=0;n<120;n++){const current=await r.tui.send('snapshot');if(current.session.agents.find(a=>a.id===agentId)?.accountProfile===primary){switched=true;break;}await sleep(250);}
 if(!switched)throw Error('Visible signed-in-account popup did not switch to the primary');
 await r.capture('signed-in-account-restored-editor');await r.tui.type('\r');await r.turn('','CX13_DRAFT_RETRY',240000,false);
 result.draftRetry='Same real editor/attachment after selecting the signed-in account; one provider turn on gpt-6-luna low';
 // The UI writes both newly serialized settings; only allowlisted keys are recorded.
 const models={codex_brief_model:'gpt-6-luna',claude_brief_model:'claude-haiku-5-5'};
 for(const [key,value]of Object.entries(models))await r.command('config set history.handoff.'+key+' '+value);
 let config=await r.client.send({GetUserConfig:null});for(const [key,value]of Object.entries(models))if(config.UserConfig.config.history.handoff?.[key]!==value)throw Error('Populated UserConfig handoff values did not roundtrip');await r.record('populated-handoff-config',{handoff:config.UserConfig.config.history.handoff});
 await r.restart();config=await r.client.send({GetUserConfig:null});for(const [key,value]of Object.entries(models))if(config.UserConfig.config.history.handoff?.[key]!==value)throw Error('Handoff config lost across kernel restart');result.handoffConfig={...models,persistedAcrossRestart:true,wireVersionAcceptance:'Requires coordinator-allocated shared protocol number and populated snapshot/hash'};
 result.status='PASS';
}catch(e){result.firstSeam=String(e?.message??e);await r.capture('draft-config-failure').catch(()=>{});}
finally{result.finished=new Date().toISOString();await r.record('result',result);await finish();}
if(result.status!=='PASS')process.exitCode=1;
