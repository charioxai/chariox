// MP-08 / MP-10 / MP-11: real Provider Accounts controls, fresh utility and isolated primary turn.
import {r,init,finish} from './common.mjs';
import {sleep} from './runtime.mjs';
const result={MP:['MP-08','MP-10','MP-11'],status:'FAIL',started:new Date().toISOString(),scope:'Independent disposable Codex account authentication and primary-account isolation; no manual provider login/logout/refresh'};
try{
 await init();await r.launch('codex','gpt-6-luna');
 await r.turn('Read the Goal paragraph in docs/INTER_PROVIDER_CONTEXT_SWITCHING_PLAN.md and cite its source path in one sentence. Do not edit files or read runtime state.','CX13_PRIMARY_BEFORE');
 const before=await r.binding();result.before=before;
 const path=process.env.CTXVAL_SECOND_CODEX_PROFILE;if(!path)throw Error('CTXVAL_SECOND_CODEX_PROFILE required');
 const label='ctx13-independent-'+Date.now();await r.command('provider accounts link codex '+label+' '+path);
 let profile;
 for(let n=0;n<120;n++){const response=await r.client.send({ListProviderAccountProfiles:{provider:'codex'}});profile=response.ProviderAccountProfilesListed?.profiles.find(p=>p.label===label);if(profile)break;await sleep(250);}
 if(!profile)throw Error('Real Provider Accounts link did not create the independent registration');
 result.secondaryProfile=profile.profile_id;result.firstUtilityRead='New registration; LinkProviderAccountProfile only registers the scope, then the real TUI explicit refresh runs the official app-server account/read';
 await r.command('provider accounts refresh codex '+label);
 let footer,settled=false;
 for(let n=0;n<480;n++){const s=await r.tui.send('snapshot');footer=s.footer;if(footer?.tone==='error'||footer?.message==='provider account refresh complete'){settled=true;break;}await sleep(250);}
 if(!settled)throw Error('Real Provider Accounts refresh did not settle');
 await r.capture('independent-account-read');result.accountRead={footer};
 const observed=await r.client.send({GetProviderAccountProfile:{provider:'codex',account_profile:profile.profile_id}});result.secondaryAuthState=observed.ProviderAccountProfile?.profile.auth_state;
 // Clear the retained refresh footer with the ordinary read-only agent control.
 await r.command('agent inspect '+(await r.tui.send('snapshot')).session.focusedAgentId);
 await r.turn('Without tools, give the same source path you just cited in one sentence.','CX13_PRIMARY_AFTER');
 result.after=await r.binding();if(result.after.nativeId!==before.nativeId)throw Error('Secondary-account refresh replaced the primary native thread');result.primaryUnaffected=true;
 if(footer?.tone==='error'){
  result.firstSeam=footer.message;
  if(/account\/read/.test(footer.message)&&/unauthorized \(401\)/.test(footer.message)){
   result.status='BLOCKED(coordinator: independent disposable Codex profile fresh account/read 401)';
   result.ownerAction='Coordinator repair or replace the independent disposable product-linked Codex profile using the approved product enrollment; no manual login/logout/refresh by this lane.';
  }else throw Error('Independent account control rejected: '+footer.message);
 }else if(result.secondaryAuthState==='authenticated')result.status='PASS';else throw Error('Independent account did not report authenticated');
}catch(e){result.firstSeam=String(e?.message??e);await r.capture('account-read-failure').catch(()=>{});}
finally{result.finished=new Date().toISOString();await r.record('result',result);await finish();}
if(result.status!=='PASS')process.exitCode=1;
