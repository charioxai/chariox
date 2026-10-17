import { reconcileCachedHistory } from "./cached-history-reconciliation.js"
// MP-08 / MP-10 / MP-11: mount a read-only previous view while ordinary attach runs.
import {createSignal,onMount,onCleanup,Show} from "solid-js"
import {useRenderer,useKeyboard} from "@opentui/solid"
import type {BoxRenderable,ScrollBoxRenderable} from "@opentui/core"
import type {TranscriptEntry} from "./cli-types.js"
import {bindCliViewScope,readClientView,subscribeClientViewInvalidation,clearClientViews,type CliCachedView} from "./client-view-cache.js"
import type {LocalIpcClient} from "./ipc.js"
import type {CliRuntimeBootstrapOptions,CliRuntimeBootstrapResult} from "./cli-runtime-bootstrap.js"
import {CharioxCliApp} from "./cli-app-root.js"
import {buildTranscriptEntryRenderable} from "./transcript-render.js"
import {recordPaneFirstFrame} from "./pane-first-frame-timing.js"
import {paintCachedTranscriptFrame} from "./client-view-frame.js"
import {createTranscriptSyntaxStyle,theme} from "./theme.js"
import {TUI_MOUSE_ENABLED} from "./terminal-mouse-policy.js"

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
// Rows of the bundled kernel history drawn before the full app mounts; older rows scroll in with the app.
const previewEntryLimit=40
type Preview={alias:string;entries:TranscriptEntry[];requests:()=>ReturnType<LocalIpcClient['requestStats']>}
export function CliCachedStartup(props:{pending:Promise<CliRuntimeBootstrapResult>;view:ReturnType<typeof createCliCachedStartup>}){
 const [ready,setReady]=createSignal<Extract<CliRuntimeBootstrapResult,{kind:'ready'}>|null>(null)
 const [painted,setPainted]=createSignal(false)
 const [failure,setFailure]=createSignal<string|null>(null)
 const [preview,setPreview]=createSignal<Preview|null>(null)
 const renderer=useRenderer()
 let disposed=false
 onCleanup(()=>{disposed=true})
 void props.pending.then(async result=>{
  if(result.kind!=='ready')return
  if(disposed)return
  const cached = props.view.cached(), session = result.bootstrap.binding?.session
  if (cached && (!session || cached.sessionId !== session.id || !session.agents.some(agent => agent.id === cached.agentId))) props.view.finish()
  if (result.bootstrap.deferred?.attachedHistory && session) {
   result.bootstrap.deferred.attachedHistory = result.bootstrap.deferred.attachedHistory.then(history => {
    const reconciled = reconcileCachedHistory(props.view.cached(), session, history)
    if (reconciled.discard) void clearClientViews()
    return reconciled.history
   })
  }
  // The full app takes ~100 ms of synchronous mounting. Draw the kernel's bundled history first,
  // read-only, then mount; nothing here waits on anything the attach has not already delivered.
  const history = session && result.bootstrap.deferred?.historyIsBundled ? await result.bootstrap.deferred.attachedHistory : null
  if (disposed) return
  if (history && session && history.sessionId === session.id) {
   const entries = history.historyEntries.filter(entry => !entry.hidden).slice(-previewEntryLimit)
   if (entries.length) {
    setPreview({alias: session.alias ?? session.id, entries, requests: () => result.bootstrap.client.requestStats()})
    await nextFrame(renderer)
    if (disposed) return
   }
  }
  result.bootstrap.isTranscriptViewCovered=()=>!painted()
  result.bootstrap.onTranscriptFrame=()=>{if(!disposed){setPainted(true);props.view.finish()}}
  setReady(result)
 }).catch(()=>{if(!disposed){props.view.finish();setFailure('Unable to attach to this session.')}})
 return <box width="100%" height="100%" flexDirection="column">
  <Show when={ready()}>{runtime=><CharioxCliApp bootstrap={runtime().bootstrap}/>}</Show>
  <Show when={!painted()}><box position="absolute" zIndex={100} top={0} left={0} width="100%" height="100%" backgroundColor={theme.background}>
   <Show when={preview()} fallback={<Show when={props.view.cached()} fallback={<text content={failure()??'Connecting…'} />}>{view=><CachedTranscript view={view()} title="Reconnecting — previous view"/>}</Show>}>{value=><CachedTranscript view={value()} requests={value().requests}/>}</Show>
  </box></Show>
 </box>
}
// Resolves once the frame that includes the current tree has been composed.
function nextFrame(renderer:ReturnType<typeof useRenderer>):Promise<void>{
 return new Promise(resolve=>{
  const done=()=>{renderer.removePostProcessFn(sample);clearTimeout(timer);setTimeout(resolve,0)}
  const sample=()=>done()
  const timer=setTimeout(done,100)
  renderer.addPostProcessFn(sample)
  renderer.requestRender()
 })
}
function CachedTranscript(props:{view:Pick<CliCachedView,'alias'|'entries'|'frame'>;title?:string;requests?:Preview['requests']}){
 const renderer=useRenderer(),syntax=createTranscriptSyntaxStyle()
 let box:BoxRenderable|undefined,scrollbox:ScrollBoxRenderable|undefined,timer:ReturnType<typeof setTimeout>|undefined
 const removeTiming=recordPaneFirstFrame(renderer,()=>Boolean((props.view.frame || box?.getChildren().length) && box && box.width > 0 && box.height > 0),()=>props.view.entries,props.requests?"preview":true,undefined,undefined,props.requests)
 useKeyboard(key=>{if(key.ctrl&&key.name==='c')process.exit(0)})
 onMount(()=>{
  if (props.view.frame) return
  for(const entry of props.view.entries.filter(entry=>!entry.hidden)){
   const rendered=buildTranscriptEntryRenderable(renderer,entry,syntax,()=>{},()=>{})
   box?.add(rendered.wrapper)
  }
  timer=setTimeout(()=>{scrollbox?.scrollTo({x:0,y:Math.max(0,scrollbox.scrollHeight-scrollbox.height)});renderer.requestRender()},0)
 })
 onCleanup(()=>{removeTiming();if(timer)clearTimeout(timer);syntax.destroy()})
 return <box width="100%" height="100%" flexDirection="column" backgroundColor={theme.background}>
  {/* The live preview has no header, so the rows do not shift when the app replaces it. */}
  <Show when={props.title}><text height={1} flexShrink={0} content={`${props.view.alias} · ${props.title}`} fg={theme.textMuted}/></Show>
  <Show when={props.view.frame} fallback={<scrollbox flexGrow={1} stickyScroll={true} stickyStart="bottom" viewportOptions={{paddingRight:0}} verticalScrollbarOptions={{visible:TUI_MOUSE_ENABLED,paddingLeft:0}} ref={value=>{scrollbox=value}}><box width="100%" flexDirection="column" ref={value=>{box=value}}/></scrollbox>}>{frame=><box flexGrow={1} width="100%" ref={value=>{box=value}} renderAfter={buffer=>{if(box)paintCachedTranscriptFrame(buffer,frame(),box)}}/>}</Show>
 </box>
}
