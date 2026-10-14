// MP-08 / MP-10 / MP-11: only mounted transcript presentation is saved.
import {createEffect,onCleanup} from "solid-js"
import type {LocalIpcClient} from "./ipc.js"
import type {RuntimeSession,TranscriptEntry} from "./cli-types.js"
import {cliViewScope,clientViewEpoch,writeClientView} from "./client-view-cache.js"
export function createCliViewPersistence(deps:{client:LocalIpcClient;session():RuntimeSession;agentId():string|null;entries():TranscriptEntry[];attached():boolean}):void{
 let timer:ReturnType<typeof setTimeout>|undefined
 createEffect(()=>{
  const session=deps.session(),agentId=deps.agentId(),entries=deps.entries(),attached=deps.attached()
  if(timer)clearTimeout(timer)
  if(!attached||!session.id||!agentId||!entries.length)return
  const generation=clientViewEpoch()
  // Use the normal renderer's current entries, preserving collapsed and hidden rows.
  // No RuntimeSession, attachment, permission interaction or queued action is persisted.
  const presentation=entries.filter(entry=>["user","assistant","tool","reasoning","turn_toggle"].includes(entry.role)).map(entry=>({...entry}))
  const sequence=Math.max(0,...presentation.flatMap(entry=>[entry.historyEntryIndex??0,...(entry.historySequenceRanges??[]).map(range=>range[1])]))
  timer=setTimeout(()=>{void (async()=>{
   const scope=await cliViewScope(deps.client)
   if(!scope||generation!==clientViewEpoch()||deps.session().id!==session.id||deps.agentId()!==agentId)return
   await writeClientView({scope,sessionId:session.id,agentId,alias:session.alias??session.id,sequence,savedAt:Date.now(),entries:presentation})
  })()},150)
 })
 onCleanup(()=>{if(timer)clearTimeout(timer)})
}
