import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { createInputCallers } from './browser-app-input-callers.mjs';

test('input callers match only the stamped trusted event and retire after handlers settle', async () => {
  let capture;
  const inputs = vm.runInNewContext(`(${createInputCallers.toString()})()`, {
    performance:{timeOrigin:100000,now:()=>50},addEventListener:(_type,listener)=>{ capture=listener; },
  });
  const stamped = {isTrusted:true,timeStamp:86300000};
  const human = {isTrusted:true,timeStamp:100};
  const forged = {isTrusted:false,timeStamp:86300000};
  inputs.begin('caller', 86400);
  for(const event of [stamped,human,forged]) capture(event);
  assert.equal(inputs.token(stamped), 'caller');
  assert.equal(stamped.timeStamp,50,'App listeners see the actual dispatch clock');
  assert.equal(inputs.token(human), undefined);
  assert.equal(inputs.token(forged), undefined);
  let release, settled = false;
  inputs.track('caller', new Promise(resolve => { release = resolve; }));
  const ending = inputs.end('caller').then(() => { settled = true; });
  await Promise.resolve();
  assert.equal(settled,false);
  assert.equal(inputs.token(stamped),undefined);
  release(); await ending;
  assert.equal(settled,true);
  await inputs.end('caller');
});

test('controller submission tags only the exact synchronous form event', async () => {
  const listeners=new Map();let seen;
  function Form() {}
  Form.prototype.requestSubmit=function(){const event={target:this};listeners.get(this)?.(event);seen=inputs.token(event);};
  const inputs=vm.runInNewContext(`(${createInputCallers.toString()})()`, {performance:{timeOrigin:0,now:()=>1},addEventListener(){},HTMLFormElement:Form});
  const form={addEventListener(_type,fn){listeners.set(this,fn);},removeEventListener(){listeners.delete(this);}};
  inputs.begin('caller',10);inputs.submit(form,'caller');
  assert.equal(seen,'caller');assert.equal(listeners.size,0);
  assert.equal(inputs.token({target:form}),undefined);
  await inputs.end('caller');assert.throws(()=>inputs.submit(form,'caller'),/expired/);
});

test('native text edits capture only synchronous events on the edited element', async () => {
  const listeners=new Map();let during,late,fail=false;
  const inputs=vm.runInNewContext(`(${createInputCallers.toString()})()`, {
    performance:{timeOrigin:0,now:()=>1},addEventListener(type,fn){listeners.set(type,fn);},
  });
  const element={ownerDocument:{execCommand(command,_ui,text){
    assert.equal(command,'insertText');assert.equal(text,'A😀B');
    const event={target:element,isTrusted:true};listeners.get('input')(event);during=inputs.token(event);
    const other={target:{},isTrusted:true};listeners.get('input')(other);assert.equal(inputs.token(other),undefined);
    if(fail)throw Error('native edit refused');return true;
  }}};
  inputs.begin('caller',10);inputs.edit(element,'A😀B','caller');assert.equal(during,'caller');
  const event={target:element,isTrusted:true};listeners.get('input')(event);late=inputs.token(event);assert.equal(late,undefined);
  fail=true;assert.throws(()=>inputs.edit(element,'A😀B','caller'),/native edit refused/);
  listeners.get('input')(event);assert.equal(inputs.token(event),undefined);
  await inputs.end('caller');assert.throws(()=>inputs.edit(element,'A😀B','caller'),/expired/);
});

test('native key defaults retain only the stamped source, and human input clears it', async () => {
  const listeners = new Map();
  const inputs = vm.runInNewContext(`(${createInputCallers.toString()})()`, {
    performance:{timeOrigin:0,now:()=>1},addEventListener(type,fn){listeners.set(type,fn);},
  });
  const target = {tagName:'INPUT'};
  const event = (type, properties = {}) => ({type,target,isTrusted:true,timeStamp:10000,...properties});
  const dispatch = value => { listeners.get(value.type)(value); return inputs.token(value); };
  inputs.begin('caller',10);
  dispatch(event('keydown'));
  dispatch(event('keypress'));
  assert.equal(dispatch(event('input',{timeStamp:50})),'caller','native input has its own clock');
  assert.equal(dispatch(event('input',{timeStamp:51})),undefined,'the default edit is consumed once');
  const prevented = event('keydown'); dispatch(prevented); prevented.defaultPrevented = true;
  assert.equal(dispatch(event('input')),undefined);
  dispatch(event('keydown')); dispatch(event('keydown',{timeStamp:50}));
  assert.equal(dispatch(event('input')),undefined,'concurrent human typing has no agent token');
  dispatch(event('keydown')); dispatch(event('keyup'));
  assert.equal(dispatch(event('input')),undefined,'the paired release ends the source');
  dispatch(event('keydown')); await inputs.end('caller');
  assert.equal(dispatch(event('input')),undefined,'retired source cannot tag delayed input');
});

test('CDP text claims its native beforeinput and synchronous transform only', async () => {
  const listeners = new Map(), timers = new Map(), element = {};
  let timer = 0;
  const inputs = vm.runInNewContext(`(${createInputCallers.toString()})()`, {
    document:{activeElement:element},performance:{timeOrigin:0,now:()=>1},
    addEventListener(type,fn){listeners.set(type,fn);},
    setTimeout(fn){timers.set(++timer,fn);return timer;},clearTimeout(id){timers.delete(id);},
  });
  const event = (type, overrides = {}) => ({type,target:element,isTrusted:true,inputType:'insertText',data:'Edit',...overrides});
  const dispatch = value => {listeners.get(value.type)(value);return inputs.token(value);};
  const flush = () => { for (const [id,fn] of timers) {timers.delete(id);fn();} };
  inputs.begin('caller',10);
  inputs.armText('Edit','caller');
  assert.equal(dispatch(event('beforeinput',{isTrusted:false})),undefined);
  assert.equal(dispatch(event('beforeinput')),'caller');
  assert.equal(dispatch(event('input',{data:'EDIT'})),'caller','the editor may synchronously transform the native edit');
  assert.equal(dispatch(event('input')),undefined,'consumed once');
  for (const type of ['keydown','pointerdown','paste','compositionstart','focusin']) {
    inputs.armText('Edit','caller'); dispatch(event(type,{timeStamp:50}));
    assert.equal(dispatch(event('beforeinput')),undefined,type+' retires the claim');
    assert.equal(dispatch(event('input')),undefined);
  }
  for (const override of [{target:{}},{data:'Human'},{inputType:'insertFromPaste'}]) {
    inputs.armText('Edit','caller');
    assert.equal(dispatch(event('beforeinput',override)),undefined);
    assert.equal(dispatch(event('input')),undefined);
  }
  inputs.armText('Edit','caller'); assert.equal(dispatch(event('beforeinput')),'caller');
  flush(); assert.equal(dispatch(event('input')),undefined,'a cancelled beforeinput cannot label a later task');
  inputs.armText('Edit','caller'); flush();
  assert.equal(dispatch(event('beforeinput')),undefined,'an unused claim expires');
  inputs.armText('Edit','caller'); inputs.clearText('other');
  assert.equal(dispatch(event('beforeinput')),'caller');
  inputs.clearText('caller'); assert.equal(dispatch(event('input')),undefined);
  inputs.armText('Edit','caller'); await inputs.end('caller');
  assert.equal(timers.size,0); assert.equal(dispatch(event('beforeinput')),undefined);
  assert.throws(()=>inputs.armText('Edit','caller'),/expired/);
});
