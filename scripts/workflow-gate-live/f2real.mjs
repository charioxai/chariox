import {Rig,boot,submit,sleep,runs,recorder,isTerminal,jq} from './lib.mjs';
const rec=recorder('f2real');
const rig=new Rig('f2real');await rig.start();await rig.session();
const link=await rig.send({LinkProviderAccountProfile:{provider:'codex',label:'wfgate-luna',path:'/root/.chariox/dev/worker-k/home/state/provider-accounts/local/codex/kaissandra-ai-reviewer-login-esqv7b7c/codex'}});
console.log('link keys',Object.keys(link));const prof=JSON.stringify(link).match(/"profile_id":"([^"]+)"/)?.[1];console.log('profile id present',!!prof);
await boot(rig,{provider:'codex',model:'gpt-6-luna',agentExtra:{effort:'low',accountProfile:prof}});
const ids=[];for(let i=0;i<3;i++){const e=await submit(rig,'run');await rig.waitFor(rig.alias,e.entry_id,isTerminal,30000);ids.push(e.entry_id);await sleep(500)}
const t=Date.now();let rs;for(;;){rs=await runs(rig);if(rs.length>=1&&rs.every(r=>['Completed','Failed','Stopped'].includes(r.status))&&rs.length===3)break;if(Date.now()-t>240000)break;await sleep(2000)}
const out={statuses:rs.map(r=>r.status),secs:(Date.now()-t)/1000};rec('real-codex-gated-chain',{result:'INFO',out});console.log(JSON.stringify(out));
await rig.stop();
