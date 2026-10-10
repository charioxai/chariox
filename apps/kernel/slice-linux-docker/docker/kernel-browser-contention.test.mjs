// MP-08/MP-10: reduced motion engages only under measured host saturation.
import test from 'node:test';import assert from 'node:assert/strict';
import {CpuContention} from './kernel-browser-contention.mjs';
test('MP-08/MP-10 contention fallback engages on saturation, holds, and releases with hysteresis',()=>{
 let at=0,busy=0,idle=0;const stat=()=>`cpu  ${busy} 0 0 ${idle} 0 0 0 0 0 0\n`;
 const c=new CpuContention({read:stat,now:()=>at});
 const step=(ms,busyShare)=>{at+=ms;busy+=ms*busyShare;idle+=ms*(1-busyShare);return c.engaged()};
 assert.equal(c.engaged(),false,'no measurement yet');
 assert.equal(step(600,.5),false,'a half-idle host keeps native motion');
 assert.equal(step(600,.95),false,'a short burst keeps native motion');
 assert.equal(step(600,.5),false);
 for(let n=0;n<4;n++)step(600,.95);
 assert.equal(step(600,.95),true,'sustained saturation engages the fallback');
 assert.equal(step(600,.2),true,'held at least ten seconds');
 for(let n=0;n<16;n++)step(600,.2);
 assert.equal(step(600,.2),false,'released after the hold and five clear seconds');
 assert.equal(new CpuContention({read:()=>{throw Error('no proc')}}).engaged(),false,'unmeasurable hosts stay native');
});
