// MP-08 / MP-10 / MP-11: bounded private presentation, separate from session authority.
import {readFile,mkdir,writeFile,rename,unlink,stat} from "node:fs/promises"
import {dirname,join} from "node:path"
import {validCachedTranscriptFrame} from "./client-view-frame-types.js"
import {preferencesPath, type CharioxPreferences, type RelayCloudProfile} from "./preferences.js"
import type {CliOptions, TranscriptEntry} from "./cli-types.js"
import type {LocalIpcClient} from "./ipc.js"
import {getKernelCloudRelayProfile,getRelayStatus} from "./relay-api.js"
export type CliViewScope = { owner: string; kernel: string }
export type CliCachedView = { scope: CliViewScope; sessionId: string; agentId: string; alias: string; sequence: number; savedAt: number; entries: TranscriptEntry[]; frame?: import("./client-view-frame-types.js").CachedTranscriptFrame }
const destination=()=>join(dirname(preferencesPath()),"client-cache","views.json")
const maxBytes=16*1024*1024
let epoch=0,writes:Promise<void>=Promise.resolve()
const invalidated=new Set<()=>void>()
export const subscribeClientViewInvalidation=(listener:()=>void)=>{invalidated.add(listener);return()=>{invalidated.delete(listener)}}
export const clientViewEpoch=()=>epoch
const clientScopes=new WeakMap<LocalIpcClient,{generation:number;scope:Promise<CliViewScope|null>}>()
export function cliViewOwner(profile: RelayCloudProfile|null|undefined): string {
 return profile ? JSON.stringify([profile.apiUrl,profile.userId,profile.accountId,profile.realmId]) : `local:${process.getuid?.() ?? "user"}`
}
export function bindCliViewScope(client:LocalIpcClient,options:CliOptions,preferences:CharioxPreferences):Promise<CliViewScope|null>{
 const generation=epoch
 const scope=(async()=>{
  if(options.relayUrl){const profile=preferences.relay?.cloud;return profile&&options.targetDaemonId?{owner:cliViewOwner(profile),kernel:options.targetDaemonId}:null}
  const [profile,status]=await Promise.all([getKernelCloudRelayProfile(client),getRelayStatus(client)])
  return status.daemon_id?{owner:cliViewOwner(profile),kernel:status.daemon_id}:null
 })().then(value=>generation===epoch?value:null).catch(()=>null)
 clientScopes.set(client,{generation,scope});return scope
}
export function cliViewClient(client: LocalIpcClient): LocalIpcClient {
 const current = (client as LocalIpcClient & { currentClient?: () => LocalIpcClient }).currentClient
 return typeof current === "function" ? current() : client
}
export function cliViewScope(client: LocalIpcClient): Promise<CliViewScope | null> {
 const record = clientScopes.get(cliViewClient(client))
 return record?.generation === epoch ? record.scope : Promise.resolve(null)
}
export async function clearClientViews():Promise<void>{epoch++;for(const listener of invalidated)listener();await writes.catch(()=>{});await unlink(destination()).catch(()=>{})}
export async function readClientView(scope:CliViewScope,sessionId:string,agentId?:string|null):Promise<CliCachedView|null>{
 const generation=epoch
 try{
  const info=await stat(destination());if(info.size>maxBytes||(info.mode&0o077)!==0)return null
  const rows=JSON.parse(await readFile(destination(),"utf8")) as CliCachedView[]
  if(!Array.isArray(rows)||generation!==epoch)return null
  return rows.filter(row=>valid(row)&&row.scope.owner===scope.owner&&row.scope.kernel===scope.kernel&&row.sessionId===sessionId&&(!agentId||row.agentId===agentId)).sort((a,b)=>b.savedAt-a.savedAt)[0]??null
 }catch{return null}
}
export function writeClientView(view:CliCachedView):Promise<void>{
 const generation=epoch
 writes=writes.catch(()=>{}).then(async()=>{
  if(generation!==epoch||!valid(view)||Buffer.byteLength(JSON.stringify(view))>2*1024*1024)return
  let rows:CliCachedView[]=[]
  try{if((await stat(destination())).size<=maxBytes){const old=JSON.parse(await readFile(destination(),"utf8"));if(Array.isArray(old))rows=old.filter(valid)}}catch{}
  const same=(row:CliCachedView)=>row.scope.kernel===view.scope.kernel&&row.sessionId===view.sessionId&&row.agentId===view.agentId
  if(rows.some(row=>row.scope.owner===view.scope.owner&&same(row)&&row.savedAt>view.savedAt))return
  rows=rows.filter(row=>row.scope.owner===view.scope.owner&&!same(row)).sort((a,b)=>a.savedAt-b.savedAt);rows.push(view)
  while(rows.length>32||Buffer.byteLength(JSON.stringify(rows))>maxBytes)rows.shift()
  const file=destination(),temporary=file+"."+process.pid+".tmp"
  await mkdir(dirname(file),{recursive:true,mode:0o700});await writeFile(temporary,JSON.stringify(rows),{mode:0o600})
  if(generation!==epoch){await unlink(temporary).catch(()=>{});return}await rename(temporary,file)
 }).catch(()=>{})
 return writes
}
function valid(row:CliCachedView):boolean{
 return Boolean(row&&row.scope&&[row.scope.owner,row.scope.kernel,row.sessionId,row.agentId].every(value=>typeof value==="string"&&value.length>0&&value.length<=4096)
  &&typeof row.alias==="string"&&Number.isSafeInteger(row.sequence)&&row.sequence>=0&&Number.isFinite(row.savedAt)&&Array.isArray(row.entries)&&row.entries.length<=2000
  &&(row.frame === undefined || validCachedTranscriptFrame(row.frame))
  &&row.entries.every(entry=>entry&&["user","assistant","tool","reasoning","turn_toggle"].includes(entry.role)&&typeof entry.text==="string"&&Number.isFinite(entry.id)))
}
