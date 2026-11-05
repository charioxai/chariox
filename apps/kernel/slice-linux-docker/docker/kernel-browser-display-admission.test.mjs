import test from 'node:test';
import assert from 'node:assert/strict';
import {reserveDisplay,maxDisplays} from './kernel-browser-display-admission.mjs';

const watching=n=>({displays:new Map(Array.from({length:n},(_,viewer)=>[`viewer-${viewer}`,{}]))});

// MP-08 TR-19: every viewer of a shared room holds one display, so twenty viewers must all be admitted.
test('MP-08 TR-19 a shared room admits a display for each of twenty viewers',async()=>{
 const host=watching(19);
 assert.equal(await reserveDisplay(host,async()=>'started'),'started');
});

test('MP-08 TR-19 the display bound still refuses the viewer past it',async()=>{
 const host=watching(maxDisplays);
 await assert.rejects(reserveDisplay(host,async()=>'started'),{code:'user_domain_stale_reference'});
});
