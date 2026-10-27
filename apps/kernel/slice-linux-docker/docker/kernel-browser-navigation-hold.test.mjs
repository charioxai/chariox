// MP-08/MP-10 BROWSER-UX-09: the owner's window display never shows Chrome's unpainted new document.
import test from 'node:test';
import assert from 'node:assert/strict';
import {NavigationHold} from './kernel-browser-navigation-hold.mjs';
import {WindowPageCapture,windowCaptureArgs} from './kernel-browser-window-display.mjs';

const life=(name,frameId='tab-a',loaderId='L1')=>({method:'Page.lifecycleEvent',sessionId:'s',params:{frameId,loaderId,name}});
const hold=(over={})=>{let active='tab-a',released=0;const h=new NavigationHold({active:async()=>active,release:()=>released++,...over});return {h,released:()=>released,switchTo:t=>{active=t;}};};

test('a new document holds the window display until its first paint and its parse (stale tiles of the old page are gone)',async()=>{
 const {h,released}=hold();
 assert.equal(await h.holding(),false);
 h.event(life('init'));assert.equal(await h.holding(),true);
 h.event(life('firstContentfulPaint','tab-a','OLD'));h.event(life('DOMContentLoaded','tab-a','OLD'));assert.equal(await h.holding(),true,'the old document\'s events do not release it');
 h.event(life('firstContentfulPaint'));assert.equal(await h.holding(),true,'painted but old tiles may remain');
 h.event(life('firstPaint'));h.event(life('DOMContentLoaded'));assert.equal(await h.holding(),false);assert.equal(released(),1);
 h.close();
});
test('a Tab behind the front Tab never holds the window display',async()=>{
 const {h}=hold();h.event(life('init','tab-b','L9'));assert.equal(await h.holding(),false);
 h.event(life('init','tab-a'));assert.equal(await h.holding(),true);h.close();
});
test('a document that never paints releases at load or the bound',async()=>{
 for(const name of ['load']){const {h}=hold();h.event(life('init'));h.event(life(name));assert.equal(await h.holding(),false,name);h.close();}
 const {h,released}=hold({maxMs:20});h.event(life('init'));await new Promise(r=>setTimeout(r,60));
 assert.equal(await h.holding(),false,'bounded: never fail closed');assert.equal(released(),1);h.close();
});
test('another navigation of the same Tab restarts the hold',async()=>{
 const {h}=hold();h.event(life('init','tab-a','L1'));h.event(life('init','tab-a','L2'));
 h.event(life('load','tab-a','L1'));assert.equal(await h.holding(),true);
 h.event(life('load','tab-a','L2'));assert.equal(await h.holding(),false);h.close();
});

// Through the real publish path: the white, half-painted frame between commit and first paint is dropped; the
// first frame after the paint is published (and the source is woken for it).
function capture(){
 const window={xid:42,bounds:{width:2560,height:1774},page_rect:{left:0,top:174,width:2560,height:1600}};
 const host={tabGroups:{active:async()=>'tab-a'},tabs:new Map(),protection:{targets:[]},browser:{fillTargets:new Map()}};
 const c=new WindowPageCapture({host,...windowCaptureArgs(window),isCurrent:()=>true,validateCurrent:async()=>true,occluded:async()=>false,
  tab:{tab_id:'t',document_id:'d'},scale:2,connection:{isOpen:()=>true,subscribe:()=>()=>{}},display:{},pid:42,policy:host.protection,browser:host.browser,allowed:()=>true,timing:()=>{}});
 c.valid=()=>!c.closed;c.attested=true;c.regions=c.protection();let wakes=0;c.wake=()=>wakes++;
 return {c,wakes:()=>wakes};
}
const raw=async serial=>{await new Promise(r=>setTimeout(r,3));return {width:4,height:4,length:64,pixels:Buffer.alloc(64,255),serial,format:'bgr0',captured_ms:performance.timeOrigin+performance.now()};};
const published=async(c,serial)=>{c.pending=await raw(serial);await c.publish();return c.sample()?.raw.serial??null;};
test('the window display publishes no frame between a document\'s commit and its first paint',async()=>{
 const {c}=capture();
 assert.equal(await published(c,1),null,'first publish only fences (readback precedes the fence)');
 assert.equal(await published(c,2),2,'settled page: published');
 c.onCdp(life('init'));
 assert.equal(await published(c,3),2,'white frame with leftovers of the old page: the last good frame stays');
 assert.equal(await published(c,4),2);
 c.onCdp(life('firstContentfulPaint'));assert.equal(await published(c,5),2,'painted, not parsed: still held');
 c.onCdp(life('DOMContentLoaded'));
 assert.equal(await published(c,6),6,'first painted frame is published');
 await c.close();
});
test('the first paint wakes the source so a still page is not left on the old frame',async()=>{
 const {c,wakes}=capture();c.onCdp(life('init'));const before=wakes();c.onCdp(life('firstContentfulPaint'));c.onCdp(life('DOMContentLoaded'));
 assert.equal(wakes(),before+1);await c.close();
});
test('a window display not yet attested is never held by a navigation',async()=>{
 const {c}=capture();c.attested=false;c.onCdp(life('init'));assert.equal(await c.occluded(),false);await c.close();
});
