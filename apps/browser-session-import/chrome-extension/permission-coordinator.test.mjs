import assert from 'node:assert/strict';
import test from 'node:test';
import {PermissionGrantCoordinator,createChromePermissionLifecycle,
  snapshotPermissionState} from './permission-coordinator.mjs';

const permission = {permissions:['cookies'],origins:['*://example.com/*','*://login.example.com/*']};

function sharedSessionState() {
  let state;
  return {
    async load() { return state === undefined ? undefined : structuredClone(state); },
    async save(value) { state = structuredClone(value); },
    inspect() { return state; },
  };
}

test('snapshot records cookies and every exact host independently', async () => {
  const queries = [];
  const chrome = {permissions:{contains:async value => {
    queries.push(value);
    return value.permissions?.[0] === 'cookies' || value.origins?.[0] === '*://example.com/*';
  }}};
  assert.deepEqual(await snapshotPermissionState(chrome,permission),{
    cookies:true,origins:{'*://example.com/*':true,'*://login.example.com/*':false}});
  assert.deepEqual(queries,[{permissions:['cookies']},{origins:['*://example.com/*']},
    {origins:['*://login.example.com/*']}]);
});

test('preexisting grants are never revoked and operation-acquired grants are', async () => {
  const removed = [];
  const coordinator = new PermissionGrantCoordinator(async value => {removed.push(value); return true;});
  await coordinator.reserve('one',permission,{cookies:true,
    origins:{'*://example.com/*':true,'*://login.example.com/*':false}});
  await coordinator.activate('one');
  await coordinator.release('one');
  assert.deepEqual(removed,[{origins:['*://login.example.com/*']}]);
});

test('concurrent pages retain shared transient grants until the final operation releases', async () => {
  const removed = [];
  const coordinator = new PermissionGrantCoordinator(async value => {removed.push(value); return true;});
  const none = {cookies:false,origins:{'*://example.com/*':false,'*://login.example.com/*':false}};
  const observed = {cookies:true,origins:{'*://example.com/*':true,'*://login.example.com/*':true}};
  await coordinator.reserve('first',permission,none);
  await coordinator.activate('first');
  await coordinator.reserve('second',permission,observed);
  await coordinator.activate('second');
  await coordinator.release('first');
  assert.deepEqual(removed,[]);
  await coordinator.release('second');
  assert.deepEqual(removed,[{permissions:['cookies']},{origins:['*://example.com/*']},
    {origins:['*://login.example.com/*']}]);
});

test('denial before activation releases reservation without revoking anything', async () => {
  const removed = [];
  const coordinator = new PermissionGrantCoordinator(async value => {removed.push(value); return true;});
  await coordinator.reserve('denied',permission,{cookies:false,origins:{'*://example.com/*':false,
    '*://login.example.com/*':false}});
  await coordinator.release('denied');
  assert.deepEqual(removed,[]);
});

test('permission-added observation cleans a grant when its page closes before activation acknowledgement', async () => {
  const removed = [];
  const coordinator = new PermissionGrantCoordinator(async value => {removed.push(value); return true;});
  await coordinator.reserve('closing',permission,{cookies:false,origins:{'*://example.com/*':false,
    '*://login.example.com/*':false}});
  await coordinator.observeAdded(permission);
  await coordinator.release('closing');
  assert.deepEqual(removed,[{permissions:['cookies']},{origins:['*://example.com/*']},
    {origins:['*://login.example.com/*']}]);
});

test('release followed by late permission-added observation cleans each acquired grant at most once', async () => {
  const removed = [];
  const coordinator = new PermissionGrantCoordinator(async value => {removed.push(value); return true;});
  await coordinator.reserve('closing',permission,{cookies:false,origins:{'*://example.com/*':false,
    '*://login.example.com/*':false}});
  await coordinator.release('closing');
  assert.deepEqual(removed,[]);
  await coordinator.observeAdded(permission);
  assert.deepEqual(removed,[{permissions:['cookies']},{origins:['*://example.com/*']},
    {origins:['*://login.example.com/*']}]);
  await coordinator.observeAdded(permission);
  assert.equal(removed.length,3);
});

test('disconnect release before late permission-added preserves an overlapping active lease', async () => {
  const removed = [];
  const coordinator = new PermissionGrantCoordinator(async value => {removed.push(value); return true;});
  const none = {cookies:false,origins:{'*://example.com/*':false,'*://login.example.com/*':false}};
  const granted = {cookies:true,origins:{'*://example.com/*':true,'*://login.example.com/*':true}};
  await coordinator.reserve('disconnecting',permission,none);
  await Promise.all([coordinator.release('disconnecting'),coordinator.release('disconnecting')]);
  await coordinator.reserve('active',permission,granted);
  await coordinator.activate('active');
  await coordinator.observeAdded(permission);
  assert.deepEqual(removed,[]);
  await coordinator.release('active');
  assert.deepEqual(removed,[{permissions:['cookies']},{origins:['*://example.com/*']},
    {origins:['*://login.example.com/*']}]);
});

test('late permission-added never turns a released preexisting grant into transient ownership', async () => {
  const removed = [];
  const coordinator = new PermissionGrantCoordinator(async value => {removed.push(value); return true;});
  await coordinator.reserve('preexisting',permission,{cookies:true,origins:{'*://example.com/*':true,
    '*://login.example.com/*':true}});
  await coordinator.release('preexisting');
  await coordinator.observeAdded(permission);
  assert.deepEqual(removed,[]);
});

test('worker restart preserves an active lease for over 30 seconds and terminal release revokes it', async () => {
  const removed = [];
  const stateStore = sharedSessionState();
  const none = {cookies:false,origins:{'*://example.com/*':false,'*://login.example.com/*':false}};
  const firstWorker = new PermissionGrantCoordinator(
    async value => { removed.push(value); return true; },{stateStore,now:() => 1_000});
  await firstWorker.reserve('long-running',permission,none,{ownerTabId:71});
  await firstWorker.activate('long-running');
  assert.deepEqual(stateStore.inspect().operations['long-running'].preexisting,[]);
  assert.equal(stateStore.inspect().operations['long-running'].active,true);

  const restartedWorker = new PermissionGrantCoordinator(
    async value => { removed.push(value); return true; },{stateStore,now:() => 32_500});
  await restartedWorker.recover({ownerAlive:async tabId => tabId === 71});
  assert.deepEqual(removed,[]);
  await restartedWorker.release('long-running',71);
  assert.deepEqual(removed,[{permissions:['cookies']},{origins:['*://example.com/*']},
    {origins:['*://login.example.com/*']}]);
});

test('restart plus release before late onAdded retains ownership metadata and cleans the race', async () => {
  const removed = [];
  const stateStore = sharedSessionState();
  const none = {cookies:false,origins:{'*://example.com/*':false,'*://login.example.com/*':false}};
  const firstWorker = new PermissionGrantCoordinator(
    async value => { removed.push(value); return true; },{stateStore});
  await firstWorker.reserve('late-after-restart',permission,none,{ownerTabId:72});

  const restartedWorker = new PermissionGrantCoordinator(
    async value => { removed.push(value); return true; },{stateStore});
  await restartedWorker.release('late-after-restart',72);
  assert.deepEqual(removed,[]);
  await restartedWorker.observeAdded(permission);
  assert.deepEqual(removed,[{permissions:['cookies']},{origins:['*://example.com/*']},
    {origins:['*://login.example.com/*']}]);
  assert.doesNotMatch(JSON.stringify(stateStore.inspect()),/cookie_value|session_secret/);
});

test('restart recovery releases only a closed page while a concurrent page retains shared grants', async () => {
  const removed = [];
  const stateStore = sharedSessionState();
  const none = {cookies:false,origins:{'*://example.com/*':false,'*://login.example.com/*':false}};
  const granted = {cookies:true,origins:{'*://example.com/*':true,'*://login.example.com/*':true}};
  const firstWorker = new PermissionGrantCoordinator(
    async value => { removed.push(value); return true; },{stateStore});
  await firstWorker.reserve('closed-page',permission,none,{ownerTabId:80});
  await firstWorker.activate('closed-page');
  await firstWorker.reserve('open-page',permission,granted,{ownerTabId:81});
  await firstWorker.activate('open-page');
  const restartedWorker = new PermissionGrantCoordinator(
    async value => { removed.push(value); return true; },{stateStore});
  await restartedWorker.recover({ownerAlive:async tabId => tabId === 81});
  assert.deepEqual(removed,[]);
  await restartedWorker.release('open-page',81);
  assert.deepEqual(removed,[{permissions:['cookies']},{origins:['*://example.com/*']},
    {origins:['*://login.example.com/*']}]);
});

test('page lifecycle reconnects with the same operation id after a worker suspension', async () => {
  const messages = [];
  const ports = [];
  const makePort = () => {
    const messageListeners = [];
    const disconnectListeners = [];
    const port = {
      onMessage:{addListener(value) { messageListeners.push(value); }},
      onDisconnect:{addListener(value) { disconnectListeners.push(value); }},
      postMessage(value) {
        messages.push(value);
        queueMicrotask(() => messageListeners.forEach(listener => listener({id:value.id,ok:true})));
      },
      disconnect() {},
      suspend() { disconnectListeners.forEach(listener => listener()); },
    };
    ports.push(port);
    return port;
  };
  const chrome = {
    permissions:{contains:async () => false},
    runtime:{connect:() => makePort()},
  };
  const lifecycle = createChromePermissionLifecycle(chrome,{operationId:'fixed-operation'});
  await lifecycle.reserve(permission);
  ports[0].suspend();
  await lifecycle.activate();
  await lifecycle.release();
  assert.equal(ports.length,2);
  assert.deepEqual(messages.map(value => [value.kind,value.operation_id]),[
    ['reserve','fixed-operation'],['activate','fixed-operation'],['release','fixed-operation']]);
});
