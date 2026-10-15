// MP-08/MP-10/MP-11: native window foreground claims.
import test from 'node:test';import assert from 'node:assert/strict';
import {WindowForeground} from './kernel-browser-foreground.mjs';
test('MP-11 a foreground claim is retired by another claim or any host foreground change',()=>{
 const fg=new WindowForeground();
 const a=fg.claim('tab-a');assert(fg.holds('tab-a',a));
 const b=fg.claim('tab-b');assert(!fg.holds('tab-a',a));assert(fg.holds('tab-b',b));
 fg.reset();assert(!fg.holds('tab-b',b));
 const again=fg.claim('tab-b');assert(!fg.holds('tab-b',b),'an old source never revives on a re-claim');assert(fg.holds('tab-b',again));
});
