// 2026-10-31: a lost Chrome came back after ~250 ms every time, with nobody asking, and without a bound.
import test from 'node:test';
import assert from 'node:assert/strict';
import {BrowserRecovery} from './kernel-browser-recovery.mjs';

function fixture(t){
 t.mock.timers.enable({apis:['setTimeout','Date']});
 const log=[];let starts=0;
 const host={stateSignal:{notify:()=>log.push('notify')},timing:name=>log.push(name),start:async()=>{starts++;}};
 const recovery=new BrowserRecovery(host);
 const connection={isOpen:()=>true,subscribe:()=>()=>{}};
 return {recovery,log,starts:()=>starts,connection,notifies:()=>log.filter(entry=>entry==='notify').length,
  tick:async ms=>{t.mock.timers.tick(ms);await Promise.resolve();}};
}

test('a Chrome that dies again right after each restart is refused for growing waits up to the cap',async t=>{
 const f=fixture(t);f.recovery.watch(f.connection);
 const waits=[];
 for(let n=0;n<10;n++){
  f.recovery.schedule();
  const expected=Math.min(30_000,250*2**n);
  await f.tick(expected-1);assert.equal(f.recovery.waiting(),true,`wait ${n} lasts ${expected} ms`);
  await f.tick(1);assert.equal(f.recovery.waiting(),false);waits.push(expected);
  f.recovery.watch(f.connection); // the restart succeeded and the browser died again at once
 }
 assert.deepEqual(waits,[250,500,1000,2000,4000,8000,16000,30000,30000,30000]);
});

test('the backoff starts over only after the browser stayed up',async t=>{
 const f=fixture(t);f.recovery.watch(f.connection);
 for(let n=0;n<3;n++){f.recovery.schedule();await f.tick(30_000);f.recovery.watch(f.connection);}
 f.recovery.schedule();await f.tick(1999);assert.equal(f.recovery.waiting(),true,'a browser that did not stay up keeps its backoff');await f.tick(1);assert.equal(f.recovery.waiting(),false);
 f.recovery.watch(f.connection);await f.tick(60_000);
 f.recovery.schedule();await f.tick(249);assert.equal(f.recovery.waiting(),true);await f.tick(1);assert.equal(f.recovery.waiting(),false,'a stable browser that dies is a fresh failure');
});

test('recovery never launches the browser by itself: it tells watchers when asking is allowed again',async t=>{
 const f=fixture(t);f.recovery.watch(f.connection);
 f.recovery.schedule();
 assert.equal(f.notifies(),1,'the loss is announced');
 assert(f.log.includes('browser_lost'));
 await f.tick(250);assert.equal(f.notifies(),2,'the end of the wait wakes the watchers');
 await f.tick(10*60_000);
 assert.equal(f.starts(),0);
});

test('an explicitly stopped browser is neither refused nor woken',async t=>{
 const f=fixture(t);f.recovery.watch(f.connection);f.recovery.schedule();f.recovery.stop();
 await f.tick(60_000);assert.equal(f.notifies(),1);
 f.recovery.schedule();assert.equal(f.notifies(),1,'a stopped recovery schedules nothing');
});
