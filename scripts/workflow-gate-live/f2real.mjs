import {Rig,boot,submit,sleep,runs,recorder,isTerminal,jq} from './lib.mjs';
const rec=recorder('f2real');
const rig=new Rig('f2real',{extraEnv:{CHARIOX_CODEX_BIN:'/usr/local/bin/codex',IS_SANDBOX:'1'}});await rig.start();await rig.session();
const link=await rig.send({LinkProviderAccountProfile:{provider:'codex',label:'wfgate-luna',path:'/root/.chariox/dev/worker-k/home/state/provider-accounts/local/codex/kaissandra-ai-reviewer-login-esqv7b7c/codex'}});
console.log('link keys',Object.keys(link));const prof=JSON.stringify(link).match(/"profile_id":"([^"]+)"/)?.[1];console.log('profile id present',!!prof);
await boot(rig,{provider:'codex',model:'gpt-6-luna',agentExtra:{effort:'low',accountProfile:prof}});
const N=+(process.env.F2_N||3);const ids=[];for(let i=0;i<N;i++){const e=await submit(rig,'run');await rig.waitFor(rig.alias,e.entry_id,isTerminal,30000);ids.push(e.entry_id);await sleep(500)}
const TREE=new URL('../../',import.meta.url).pathname.replace(/\/$/,'');const requests=await import(TREE+'/packages/kernel-client/dist/ipc-requests.js');
const launched=new Set();const t=Date.now();let rs;for(;;){rs=await runs(rig);for(const r of rs.filter(x=>x.status==='Created'||x.status==='Running')){for(const ag of Object.values(r.runtime_agent_ids_by_node??{})){if(launched.has(ag))continue;launched.add(ag);try{const l=await rig.send(requests.launchProviderRunsRequest([{sessionId:rig.sessionId,provider:'codex',accountProfile:prof,effort:'low',model:'gpt-6-luna',agentId:ag,native:{nativeTui:true}}],1));console.log('launch',JSON.stringify(l).slice(0,300))}catch(e){console.log('launch err',String(e).slice(0,300))}}}if(rs.length>=1&&rs.every(r=>['Completed','Failed','Stopped'].includes(r.status))&&rs.length===N)break;if(Date.now()-t>240000)break;await sleep(2000)}
const out={statuses:rs.map(r=>r.status),secs:(Date.now()-t)/1000};rec('real-codex-gated-chain',{result:'INFO',out});console.log(JSON.stringify(out));
await rig.stop();
