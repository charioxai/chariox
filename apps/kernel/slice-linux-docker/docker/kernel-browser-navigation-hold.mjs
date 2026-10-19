// MP-08/MP-10 (BROWSER-UX-09): from the moment a Tab's main frame starts a new document, Chrome's own window shows the
// unpainted area white, then tiles of the old page's header until the new document's own tiles replace them (measured:
// first paint plus DOMContentLoaded). The owner's window display keeps its last good frame for that interval; it never
// waits longer than `maxMs` (a document that never paints) and `load` always ends it.
export class NavigationHold {
 constructor({active,release=()=>{},maxMs=4000}){Object.assign(this,{activeTarget:active,onRelease:release,maxMs});this.holds=new Map();}
 // The browser connection carries every attached page session; `frameId` is the Tab target for a main frame.
 event(message){
  if(message?.method!=='Page.lifecycleEvent')return;
  const {frameId,loaderId,name}=message.params??{};
  if(typeof frameId!=='string')return;
  if(name==='init'){
   clearTimeout(this.holds.get(frameId)?.timer);
   const timer=setTimeout(()=>this.end(frameId),this.maxMs);timer.unref?.();
   this.holds.set(frameId,{loaderId,timer,seen:new Set()});
   return;
  }
  const hold=this.holds.get(frameId);
  if(hold?.loaderId!==loaderId)return;
  if(name==='firstPaint'||name==='firstContentfulPaint')hold.seen.add('paint');
  else if(name==='DOMContentLoaded')hold.seen.add('parsed');
  else if(name!=='load')return;
  if(name==='load'||hold.seen.size===2)this.end(frameId);
 }
 end(frameId){
  const hold=this.holds.get(frameId);if(!hold)return;
  clearTimeout(hold.timer);this.holds.delete(frameId);this.onRelease();
 }
 // Only the Tab the person is looking at holds the window display.
 async holding(){
  if(!this.holds.size)return false;
  return this.holds.has(await this.activeTarget());
 }
 close(){for(const {timer} of this.holds.values())clearTimeout(timer);this.holds.clear();}
}
