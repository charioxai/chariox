// MP-08/MP-10/MP-11: coalesced refresh keeps fresh ownership and close fencing.
import test from 'node:test';import assert from 'node:assert/strict';
import {DesktopSource} from './kernel-desktop-source.mjs';
function fixture(){
 const state={processes:[],browser_processes:[],serial:0,mask:false,values:[]};
 const source=new DesktopSource({}, {unknown:false,values:[]},{native:null});source.current=state;
 const notifications=[];source.worker={notify:value=>notifications.push(value)};
 return {source,state,notifications};
}
test('MP-08/MP-10/MP-11 ownership refresh is single flight and coalesces to one final fresh scope',async()=>{
 const {source,state,notifications}=fixture();let release,active=0,max=0,calls=0;
 const hold=new Promise(r=>release=r);source.scope=async()=>{active++;max=Math.max(max,active);if(++calls===1)await hold;active--;return state};
 const first=source.wake(),second=source.wake();source.wake();release();await Promise.all([first,second]);
 assert.equal(max,1);assert.equal(calls,2);assert.equal(notifications.length,2);
});
test('MP-11 close during scope discovery refuses late refresh and queued hints',async()=>{
 const {source,state,notifications}=fixture();let release,calls=0;const hold=new Promise(r=>release=r);
 source.scope=async()=>{calls++;await hold;return state};const pending=source.wake();source.wake();source.closed=true;release();await pending;
 assert.equal(calls,1);assert.equal(notifications.length,0);
});
test('MP-08/MP-10 one observation pass returns both process sets while legacy bindings stay supported',async()=>{
 let calls=0;const source=new DesktopSource({renderProcesses:async()=>{calls++;return {processes:[],browser_processes:[]}},ownedProcesses:()=>assert.fail('global scan'),browserProcesses:()=>assert.fail('second scan')},{unknown:false,values:[]},{native:null});
 await source.scope();assert.equal(calls,1);
 const legacy=new DesktopSource({ownedProcesses:async()=>[],browserProcesses:async()=>[]},{unknown:false,values:[]},{native:null});assert.deepEqual((await legacy.scope()).processes,[]);
});

test('MP-08/MP-10 stable protected input wake reaches the native worker before ownership discovery',async()=>{
 const {source,state,notifications}=fixture();source.gate.current={serial:1,measurement:{pages:[]}};Object.assign(state,{serial:1,browser_protection:source.gate.protection});
 let release;const hold=new Promise(r=>release=r);source.scope=async()=>{await hold;return state};const pending=source.wake();
 const early=notifications.length;release();await pending;assert.equal(early,1,'stable input wake waited for process discovery');
});
test('MP-11 a changed protection serial cannot issue the immediate native wake',async()=>{
 const {source,state,notifications}=fixture();source.gate.current={serial:2,measurement:{pages:[]}};Object.assign(state,{serial:1,browser_protection:{pages:[]}});
 let release;const hold=new Promise(r=>release=r);source.scope=async()=>{await hold;return state};const pending=source.wake();
 const early=notifications.length;release();await pending;assert.equal(early,0);
});
