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
    performance:{timeOrigin:0,now:()=>1},addEventListener(type,fn){listeners.set(type,fn);},setTimeout(){},
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

function textFixture() {
  const listeners = new Map(), timers = new Map();
  const element = {value:'',selectionStart:0,selectionEnd:0,selectionDirection:'none',isConnected:true};
  const document = {activeElement:element};
  let timer = 0;
  const inputs = vm.runInNewContext(`(${createInputCallers.toString()})()`, {
    document,performance:{timeOrigin:0,now:()=>1},
    addEventListener(type,fn){listeners.set(type,fn);},
    setTimeout(fn){timers.set(++timer,fn);return timer;},clearTimeout(id){timers.delete(id);},
  });
  const event = (type, overrides = {}) => ({type,target:element,isTrusted:true,timeStamp:50,inputType:'insertText',data:'Edit',
    preventDefault(){this.defaultPrevented = true;},stopImmediatePropagation(){this.stopped = true;},...overrides});
  const dispatch = value => {listeners.get(value.type)?.(value);return inputs.token(value);};
  const insert = (transform = text => text, text = 'Edit') => {
    const before = event('beforeinput',{data:text}), caller = dispatch(before);
    if (before.defaultPrevented) return {before,caller};
    element.value += transform(before.data);
    const input = event('input',{data:transform(before.data)});
    return {before,caller,input,after:dispatch(input)};
  };
  const flush = () => { for (const [id,fn] of [...timers]) {timers.delete(id);fn();} };
  inputs.begin('caller',10);
  return {inputs,element,document,event,dispatch,insert,flush,timers};
}

test('queued text checks state rather than physical event classes', () => {
  const f = textFixture();
  for (const type of ['keydown','paste','cut','drop','compositionstart','compositionupdate','compositionend',
    'focusout','focusin','pointerdown','pointerup','pointermove','mousedown','mouseup','mousemove','click','dblclick','keyup','wheel']) {
    f.inputs.armText('Edit','caller');
    assert.equal(f.dispatch(f.event(type)),undefined);
    assert.equal(f.inputs.startText('caller'),true,type+' without a state change preserves insertion');
    const edit = f.insert(text => text.toUpperCase());
    assert.equal(edit.caller,'caller');assert.equal(edit.after,'caller');
    assert.equal(f.inputs.finishText('caller'),true);
    f.inputs.clearText('caller');
    assert.equal(f.dispatch(f.event('input')),undefined,'acknowledged edit cannot tag later input');
  }
});

test('competing human edits stay unscoped and refuse the queued insertion', () => {
  const f = textFixture();
  f.inputs.armText('Edit','caller');
  const human = f.insert();
  assert.equal(human.caller,undefined,'even identical human text before dispatch is unscoped');
  assert.equal(human.after,undefined);
  assert.equal(f.inputs.startText('caller'),false);
  assert.equal(f.element.value,'Edit','the stale insertion is never dispatched');
});

for (const change of [f => { f.element.value = 'changed'; }, f => { f.document.activeElement = {}; },
  f => { f.element.selectionStart = 1; }, f => { f.element.selectionEnd = 1; },
  f => { f.element.selectionDirection = 'backward'; }, f => { f.element.isConnected = false; }]) {
  test('focus/content/selection changes refuse before dispatch and at the native insertion boundary', () => {
    for (const late of [false,true]) {
      const f = textFixture();f.inputs.armText('Edit','caller');
      if (late) assert.equal(f.inputs.startText('caller'),true);
      change(f);
      const value = f.element.value;
      if (late) {
        const refused = f.insert();
        assert.equal(refused.before.defaultPrevented,true);assert.equal(refused.before.stopped,true);
        assert.equal(refused.caller,undefined);assert.equal(refused.input,undefined);
        assert.equal(f.inputs.finishText('caller'),false);
      } else assert.equal(f.inputs.startText('caller'),false);
      assert.equal(f.element.value,value);
      f.inputs.clearText('caller');assert.equal(f.timers.size,0);
    }
  });
}

test('contenteditable fingerprint includes DOM and selection node identities', () => {
  for (const mutate of [(element, selection) => {element.innerHTML = '<b>changed</b>';},
    (element, selection) => {selection.anchorNode = {};},(element, selection) => {selection.focusOffset++;}]) {
    const f = textFixture(), selection = {anchorNode:{},anchorOffset:0,focusNode:{},focusOffset:0};
    delete f.element.value;f.element.innerHTML = 'Draft';f.element.ownerDocument = {getSelection:()=>selection};
    f.inputs.armText('Edit','caller');mutate(f.element,selection);
    assert.equal(f.inputs.startText('caller'),false);
    f.inputs.clearText('caller');
  }
});

test('cancel, expiry, acknowledgement and invocation end cannot lend a text caller', async () => {
  const f = textFixture();
  f.inputs.armText('Edit','caller');assert.equal(f.inputs.startText('caller'),true);
  const before = f.event('beforeinput');assert.equal(f.dispatch(before),'caller');before.preventDefault();
  assert.equal(f.inputs.finishText('caller'),true,'native cancellation is a successful no-op');
  f.flush();assert.equal(f.dispatch(f.event('input')),undefined,'no caller crosses a task after cancellation');
  f.inputs.clearText('caller');
  f.inputs.armText('Edit','caller');f.flush();assert.equal(f.inputs.startText('caller'),false,'expired before dispatch');
  f.inputs.armText('Edit','caller');assert.equal(f.inputs.startText('caller'),true);f.flush();
  assert.equal(f.insert().before.defaultPrevented,true,'expiry during dispatch retains only the refusal fence');
  assert.equal(f.inputs.finishText('caller'),false);f.inputs.clearText('caller');
  f.inputs.armText('Edit','caller');f.inputs.clearText('other');assert.equal(f.inputs.startText('caller'),true);
  await f.inputs.end('caller');
  assert.equal(f.insert().before.defaultPrevented,true,'ended invocation cannot dispatch as human');
  assert.equal(f.inputs.finishText('caller'),false);f.inputs.clearText('caller');assert.equal(f.timers.size,0);
  assert.throws(()=>f.inputs.armText('Edit','caller'),/expired/);
});

test('an identical physical key edit during dispatch remains human and refuses the later CDP insertion', () => {
  const f = textFixture();f.inputs.armText('h','caller');assert.equal(f.inputs.startText('caller'),true);
  f.dispatch(f.event('keydown',{key:'h'}));f.dispatch(f.event('keypress',{key:'h'}));
  const human = f.insert(undefined,'h');assert.equal(human.caller,undefined);assert.equal(human.after,undefined);
  const agent = f.insert(undefined,'h');assert.equal(agent.before.defaultPrevented,true);assert.equal(agent.caller,undefined);
  assert.equal(f.element.value,'h');assert.equal(f.inputs.finishText('caller'),false);
  f.inputs.clearText('caller');
});

test('only the keypress that Chromium turns into a beforeinput marks a physical edit', () => {
  const key = (f, overrides = {}) => { f.dispatch(f.event('keydown',{key:'h',...overrides})); return f.event('keypress',{key:'h',...overrides}); };
  const agentInsert = (f, text = 'h') => {
    f.inputs.armText(text,'caller');assert.equal(f.inputs.startText('caller'),true);
    const edit = f.insert(undefined,text);assert.equal(edit.caller,'caller');assert.equal(edit.after,'caller');
    assert.equal(f.inputs.finishText('caller'),true);f.inputs.clearText('caller');
  };
  // A held key whose own edit an App cancelled leaves no claim on a later insertion.
  let f = textFixture();const press = key(f);f.dispatch(press);
  const cancelled = f.event('beforeinput',{data:'h'});f.dispatch(cancelled);cancelled.preventDefault();
  agentInsert(f);
  // Held shortcuts edit nothing, with or without a keypress.
  f = textFixture();f.dispatch(f.event('keydown',{key:'c',ctrlKey:true}));agentInsert(f,'c');
  f = textFixture();f.dispatch(key(f,{ctrlKey:true}));agentInsert(f);
  f = textFixture();f.dispatch(key(f,{metaKey:true}));agentInsert(f);
  // A keypress whose default action was cancelled, retired or on another target produces no beforeinput.
  f = textFixture();const stopped = key(f);f.dispatch(stopped);stopped.preventDefault();agentInsert(f);
  f = textFixture();f.dispatch(key(f));f.dispatch(f.event('keyup',{key:'h'}));agentInsert(f);
  f = textFixture();f.dispatch(key(f));f.flush();agentInsert(f);
  f = textFixture();f.dispatch(key(f,{target:{}}));agentInsert(f);
  // The keypress is consumed by the one beforeinput it produces.
  f = textFixture();f.inputs.armText('h','caller');assert.equal(f.inputs.startText('caller'),true);
  f.dispatch(key(f));assert.equal(f.insert(undefined,'h').caller,undefined);
  assert.equal(f.insert(undefined,'h').before.defaultPrevented,true,'the person already changed the field');
});

test('nonediting key names can be inserted literally with the queued caller', () => {
  for (const key of ['Shift','ArrowLeft','ArrowRight','Home','End','Control','Alt']) {
    const f = textFixture();f.inputs.armText(key,'caller');f.dispatch(f.event('keydown',{key}));
    assert.equal(f.inputs.startText('caller'),true);
    assert.equal(f.dispatch(f.event('beforeinput',{data:key})),'caller',key+' is text, not a physical default edit');
    f.element.value = key;assert.equal(f.dispatch(f.event('input',{data:key})),'caller');
    assert.equal(f.inputs.finishText('caller'),true);f.inputs.clearText('caller');
  }
});
