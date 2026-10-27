// MP-08/MP-10/MP-11: page-side observer that retires the cached layout verdict of a visible-page
// display. The layout viewport (scrollbars, zoom, scale) has no CDP change event, so a page-side
// script in a private isolated world reports every signal that can change it; each report runs
// before the frame that shows the change is painted, i.e. ahead of any pixel Node can publish.
// The verdict is reused only while no report arrived since a fresh read that matched, and never
// longer than MAX_AGE_MS (a bound for changes without any signal, not the proof). Any loss of the
// observer (navigation, detach, context loss, install failure) means every publication reads.
import {randomUUID} from 'node:crypto';
import {retireNativeStates} from './kernel-browser-native-worker.mjs';
export const MAX_AGE_MS=250;
// Everything that can change the layout viewport size, zoom or pinch scale: window and visual
// viewport resize (also browser zoom and DPR), root/body box changes (scrollbar appearance),
// any DOM or attribute mutation, font and image loads, CSS transitions/animations, page show.
// Scrolling alone changes none of the four values and does not retire the verdict.
export const observerSource=binding=>`(()=>{
 const call=globalThis[${JSON.stringify(binding)}],stopName=${JSON.stringify(binding+'Stop')};let queued=false,resize,mutation;
 const fire=()=>{if(queued)return;queued=true;queueMicrotask(()=>{queued=false;call('1');});};
 const abort=new AbortController(),opts={capture:true,passive:true,signal:abort.signal};
 const stop=globalThis[stopName]=()=>{abort.abort();resize?.disconnect();mutation?.disconnect();queued=true;delete globalThis[stopName];};
 try{
  const root=document.documentElement;
  for(const type of ['resize','pageshow','load','transitionrun','transitionend','animationstart','animationiteration','animationend'])addEventListener(type,fire,opts);
  visualViewport?.addEventListener('resize',fire,opts);
  resize=new ResizeObserver(fire);resize.observe(root);if(document.body)resize.observe(document.body);
  mutation=new MutationObserver(fire);mutation.observe(root,{subtree:true,childList:true,attributes:true,characterData:true});
  document.fonts?.addEventListener('loadingdone',fire,{signal:abort.signal});
  document.addEventListener('readystatechange',fire,{signal:abort.signal});
 }catch(e){stop();throw e;}
})()`;
export async function observeLayout(connection,sessionId,now=()=>performance.now()){
  if(typeof connection?.subscribe!=='function')return null;
  const world=`chariox-layout-${randomUUID()}`,binding=`charioxLayout${randomUUID().replaceAll('-','')}`;
  let generation=0,confirmed=-1,confirmedAt=0,dead=false,contextId=null,installing=true;
  const retire=()=>{generation++;retireNativeStates();};
  const kill=()=>{dead=true;retire();off();};
  // Best effort: stop the page-side observers while their context lives, then drop the binding.
  const teardown=()=>{
    if(contextId!==null)connection.send('Runtime.evaluate',{expression:`globalThis[${JSON.stringify(binding+'Stop')}]?.()`,contextId},sessionId).catch(()=>{});
    connection.send('Runtime.removeBinding',{name:binding},sessionId).catch(()=>{});
  };
  const off=connection.subscribe(message=>{
    const {method,params}=message;
    if(method==='Chariox.browserDisconnected'){kill();return;}
    if(method==='Target.detachedFromTarget'&&params?.sessionId===sessionId||message.sessionId===sessionId&&method==='Inspector.detached'){kill();return;}
    if(message.sessionId!==sessionId)return;
    if(method==='Runtime.bindingCalled'&&params?.name===binding){if(params.executionContextId===contextId||installing)retire();return;}
    // A top-level navigation or any loss of the observer's context ends it for good.
    if(method==='Page.frameNavigated'&&!params?.frame?.parentId||method==='Runtime.executionContextsCleared'||method==='Runtime.executionContextDestroyed'&&params?.executionContextId===contextId)kill();
  });
  try{
    const {frameTree}=await connection.send('Page.getFrameTree',{},sessionId);
    await connection.send('Runtime.addBinding',{name:binding,executionContextName:world},sessionId);
    const created=await connection.send('Page.createIsolatedWorld',{frameId:frameTree.frame.id,worldName:world},sessionId);
    contextId=created.executionContextId;
    const result=await connection.send('Runtime.evaluate',{expression:observerSource(binding),contextId},sessionId);
    if(result.exceptionDetails)throw Error('layout observer install');
  }catch{kill();teardown();return null;}
  installing=false;retire();
  return {
    /** True only while the last matching fresh read is still provably current. */
    valid:()=>!dead&&confirmed===generation&&now()-confirmedAt<MAX_AGE_MS,
    /** Run a fresh read; it becomes the verdict only if it matched and no report arrived meanwhile. */
    async confirm(read){
      const started=generation,at=now(),same=await read();
      if(same&&!dead&&generation===started){confirmed=started;confirmedAt=at;}
      return same;
    },
    /** Disconnect the page-side observers (while their context lives), then drop the binding. */
    close(){
      const live=!dead;off();dead=true;
      if(live)teardown();else connection.send('Runtime.removeBinding',{name:binding},sessionId).catch(()=>{});
    },
  };
}
