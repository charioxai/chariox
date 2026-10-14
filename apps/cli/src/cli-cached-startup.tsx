// MP-08 / MP-10 / MP-11: mount a read-only previous view while ordinary attach runs.
import {createSignal,onMount,onCleanup,Show} from "solid-js"
import {useRenderer,useKeyboard} from "@opentui/solid"
import type {BoxRenderable,ScrollBoxRenderable} from "@opentui/core"
import {bindCliViewScope,readClientView,subscribeClientViewInvalidation,type CliCachedView} from "./client-view-cache.js"
import type {CliRuntimeBootstrapOptions,CliRuntimeBootstrapResult} from "./cli-runtime-bootstrap.js"
import {CharioxCliApp} from "./cli-app-root.js"
import {buildTranscriptEntryRenderable} from "./transcript-render.js"
import {recordPaneFirstFrame} from "./pane-first-frame-timing.js"
import {createTranscriptSyntaxStyle,theme} from "./theme.js"

export function createCliCachedStartup(){
 const [cached,setCached]=createSignal<CliCachedView|null>(null)
 let finished=false
 const unsubscribe=subscribeClientViewInvalidation(()=>setCached(null))
 const onClientReady:NonNullable<CliRuntimeBootstrapOptions['onClientReady']>=(client,options,preferences)=>{
  if(!options.sessionId||options.createSession||options.deleteSessionRef||options.detached)return
  void bindCliViewScope(client,options,preferences).then(async scope=>{
   if(!scope)return
   const view=await readClientView(scope,options.sessionId!)
   if(!finished)setCached(view)
  }).catch(()=>{})
 }
 return {cached,onClientReady,finish:()=>{finished=true;setCached(null);unsubscribe()}}
}
export function CliCachedStartup(props:{pending:Promise<CliRuntimeBootstrapResult>;view:ReturnType<typeof createCliCachedStartup>}){
 const [ready,setReady]=createSignal<Extract<CliRuntimeBootstrapResult,{kind:'ready'}>|null>(null)
 const [failure,setFailure]=createSignal<string|null>(null)
 let disposed=false
 onCleanup(()=>{disposed=true})
 void props.pending.then(async result=>{
  if(result.kind!=='ready')return
  // Keep the previous view until the ordinary canonical hydration is available.
  const history=await result.bootstrap.deferred?.attachedHistory
  if(history&&result.bootstrap.binding){result.bootstrap.binding.historyEntries=history.historyEntries;result.bootstrap.binding.nextHistoryCursor=history.nextHistoryCursor}
  if(disposed)return
  setReady(result);props.view.finish()
 }).catch(()=>{if(!disposed){props.view.finish();setFailure('Unable to attach to this session.')}})
 return <Show when={ready()} fallback={<Show when={props.view.cached()} fallback={<text content={failure()??'Connecting…'} />}>{view=><CachedTranscript view={view()}/>}</Show>}>
  {runtime=><CharioxCliApp bootstrap={runtime().bootstrap}/>}
 </Show>
}
function CachedTranscript(props:{view:CliCachedView}){
 const renderer=useRenderer(),syntax=createTranscriptSyntaxStyle()
 let box:BoxRenderable|undefined,scrollbox:ScrollBoxRenderable|undefined,timer:ReturnType<typeof setTimeout>|undefined
 const removeTiming=recordPaneFirstFrame(renderer,()=>Boolean(box?.getChildren().length),()=>props.view.entries,true)
 useKeyboard(key=>{if(key.ctrl&&key.name==='c')process.exit(0)})
 onMount(()=>{
  for(const entry of props.view.entries.filter(entry=>!entry.hidden)){
   const rendered=buildTranscriptEntryRenderable(renderer,entry,syntax,()=>{},()=>{})
   box?.add(rendered.wrapper)
  }
  timer=setTimeout(()=>{scrollbox?.scrollTo({x:0,y:Math.max(0,scrollbox.scrollHeight-scrollbox.height)});renderer.requestRender()},0)
 })
 onCleanup(()=>{removeTiming();if(timer)clearTimeout(timer);syntax.destroy()})
 return <box width="100%" height="100%" flexDirection="column" backgroundColor={theme.background}>
  <text content={`${props.view.alias} · Reconnecting — previous view`} fg={theme.textMuted}/>
  <scrollbox flexGrow={1} ref={value=>{scrollbox=value}}><box width="100%" flexDirection="column" ref={value=>{box=value}}/></scrollbox>
 </box>
}
