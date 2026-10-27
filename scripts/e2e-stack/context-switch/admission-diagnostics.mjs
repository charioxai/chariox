import {access} from 'node:fs/promises';
// MP-08 / MP-10 / MP-11: allowlisted error classifications, no log/config/credential payloads.
import {readFile,readdir} from 'node:fs/promises';
export async function admissionDiagnostics(root,client){
 await access(root+'/context-switch-owned.json');
 const classes={missingLaunchCredential:'remote_provider_launch_credential_required',unavailableAccount:'account is unavailable on the worker',backingProfileMismatch:'leased profile differs from the backing agent',catalogRefreshPending:'native_runtime_catalog_refresh_pending',receiptRetryFence:'worker rejection receipt fences',vaultLocked:'credential vault is locked',invalidProviderState:'InvalidProviderRunState',nativeRootPermission:'dangerously-skip-permissions cannot be used with root',auth401:'401 Unauthorized',unauthorized:'unauthorized'};
 const counts=Object.fromEntries(Object.keys(classes).map(k=>[k,0])),events=[];
 const files=[root+'/kernel.private.log'];for(const f of await readdir(root+'/home/logs').catch(()=>[]))if(f.endsWith('.ndjson'))files.push(root+'/home/logs/'+f);
 for(const file of files){let text;try{text=await readFile(file,'utf8')}catch{continue}for(const line of text.split('\n')){const low=line.toLowerCase();for(const [k,value]of Object.entries(classes))if(low.includes(value.toLowerCase()))counts[k]++;let v;try{v=JSON.parse(line)}catch{continue}const error=String(v.fields?.error??'');if(error){const categories=Object.entries(classes).filter(([,s])=>error.toLowerCase().includes(s.toLowerCase())).map(([k])=>k);events.push({at:v.timestamp_ms??null,component:/^[a-z_.-]{1,120}$/.test(v.component??'')?v.component:null,knownErrorCategories:categories,operation:error.match(/local transport `([a-zA-Z0-9 _-]{1,100})`/)?.[1]??null,errorBytes:Buffer.byteLength(error)});}}}
 const sessions=(await client.send({ListSessions:null})).SessionsListed.sessions,profiles=[];
 for(const s of sessions){const state=(await client.send({GetSessionState:{session_id:s.id}})).SessionState;profiles.push({sessionId:s.id,agents:state.session.agents.map(a=>({id:a.id,provider:a.provider,model:a.model,effort:a.effort,state:a.state,is_processing:a.is_processing,nativeId:a.provider_resume_state?.claude_session_id??null}))});}
 return {scope:'allowlisted product session state + fixed error classifiers; no raw logs or account state',counts,events,profiles};
}
