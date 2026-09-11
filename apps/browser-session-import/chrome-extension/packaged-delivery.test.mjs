import assert from 'node:assert/strict';
import {once} from 'node:events';
import {mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import test from 'node:test';
import {packageChromeExtension} from './package-extension.mjs';

const requestId = 'a'.repeat(32);
const selection = {session_id:'room',attachment_id:'attachment',environment_id:'environment',
  runtime_generation:1,tab_id:'tab',document_revision:1,source_store_id:'normal',
  domains:['example.test'],partition_sites:[],overwrite:false};

test('actual packaged confirmation click reaches encrypted relay delivery', async t => {
  const packaged = await packageTree(t);
  const {createRelayKeypair,decryptRelayPayload,encryptRelayPayload} = await load(packaged,
    'packages/kernel-client/src/browser-relay-crypto.js');
  const kernel = await createRelayKeypair();
  const wire = [];
  const metadata = [];
  let privateDelivery;
  class FixtureSocket {
    #listeners = new Map();
    bufferedAmount = 0;
    constructor() { queueMicrotask(() => this.#emit('open',{})); }
    addEventListener(kind,listener) {
      const listeners = this.#listeners.get(kind) ?? new Set();
      listeners.add(listener);
      this.#listeners.set(kind,listeners);
    }
    removeEventListener(kind,listener) { this.#listeners.get(kind)?.delete(listener); }
    close() { this.#emit('close',{}); }
    send(value) { wire.push(value); void this.#receive(value); }
    #emit(kind,event) { for (const listener of this.#listeners.get(kind) ?? []) listener(event); }
    async #receive(value) {
      const frame = JSON.parse(value);
      if (frame.kind === 'client_connect') {
        this.#emit('message',{data:JSON.stringify({kind:'client_connected',target:{daemon_id:'kernel'},
          daemon_public_key:kernel.publicKeyBase64})});
        return;
      }
      const senderKey = frame.encrypted_request.sender_public_key;
      const decoded = JSON.parse(await decryptRelayPayload(kernel.privateKey,
        frame.encrypted_request,senderKey));
      let response;
      if (decoded.browser_import_delivery) {
        privateDelivery = decoded.browser_import_delivery;
        response = {BrowserImportDelivered:{results:[
          {domain:'example.test',status:'imported',cookie_count:1},
        ]}};
      } else {
        metadata.push(decoded.request);
        const kind = Object.keys(decoded.request)[0];
        response = {BrowserImportConsent:{request_id:requestId,status:{
          PrepareBrowserImport:'prepared',ApproveBrowserImport:'approved',
          ClaimBrowserImportSource:'source_claimed',AuthorizeBrowserImportSource:'source_authorized',
          CancelBrowserImport:'cancelled'}[kind]}};
      }
      const encrypted = await encryptRelayPayload(senderKey,
        JSON.stringify({request_nonce:frame.encrypted_request.nonce,response}),kernel);
      this.#emit('message',{data:JSON.stringify({kind:'client_response',request_id:frame.request_id,
        encrypted_response:encrypted.payload,error:null})});
    }
  }
  const elements = new Map();
  const element = id => {
    if (!elements.has(id)) elements.set(id,new FixtureElement());
    return elements.get(id);
  };
  const permissionMessages = [];
  const bridgeMessages = [];
  const bridgeListeners = new Set();
  let permissionsGranted = false;
  const portMessages = new Set();
  const portDisconnects = new Set();
  const sourceCookie = {name:'session',value:'fixture-secret',domain:'example.test',path:'/',
    hostOnly:true,secure:true,httpOnly:true,session:true,sameSite:'lax',storeId:'0'};
  const browserSelection = {...selection,source_store_id:'0'};
  const globals = replaceGlobals({WebSocket:FixtureSocket,
    location:{href:'chrome-extension://fixture/connector.html?sourceTabId=7'},
    navigator:{clipboard:{writeText:async () => {}}},addEventListener:() => {},
    document:{getElementById:element,createElement:() => new FixtureElement()},chrome:{
      tabs:{get:async () => ({id:7,url:'https://example.test/private',incognito:false})},
      permissions:{request:async () => { permissionsGranted = true; return true; },
        contains:async () => permissionsGranted},
      cookies:{getAllCookieStores:async () => [{id:'0',tabIds:[7]}],
        getAll:async () => [structuredClone(sourceCookie)]},
      runtime:{connect:options => options?.name === 'browser-import-web-session-v1'
        ? {onMessage:{addListener:value => bridgeListeners.add(value)},onDisconnect:{addListener:() => {}},
          postMessage:value => bridgeMessages.push(value),disconnect:() => {}}
        : {onMessage:{addListener:value => portMessages.add(value)},
          onDisconnect:{addListener:value => portDisconnects.add(value)},
          postMessage:value => { permissionMessages.push(value.kind);
            queueMicrotask(() => { for (const listener of portMessages) listener({id:value.id,ok:true}); }); },
          disconnect:() => { for (const listener of portDisconnects) listener(); }}},
    }});
  t.after(globals.restore);
  await load(packaged,'apps/browser-session-import/chrome-extension/connector.mjs');
  await waitFor(() => bridgeMessages.some(value => value.type === 'browser_import.connector_ready.v1'));
  const ready = bridgeMessages.find(value => value.type === 'browser_import.connector_ready.v1');
  const web = await createRelayKeypair();
  const expiresAtMs = Date.now() + 100_000;
  const binding = {request_id:requestId,operation_nonce:'b'.repeat(32),
    connector_session_id:ready.connector_session_id,target_binding:'target-binding',session_id:'room',
    daemon_id:'kernel',kernel_public_key:kernel.publicKeyBase64,expires_at_ms:expiresAtMs};
  const bootstrap = {version:1,bootstrap_id:requestId,
    connector_sender_public_key:ready.connector_sender_public_key,
    expires_at_ms:expiresAtMs,relay_url:'wss://relay.fixture/ws',daemon_id:'kernel',
    kernel_public_key:kernel.publicKeyBase64,protocol_version:321,
    delivery_capability:{name:'browser_import_final_delivery',version:1},
    source:{current_profile:true,hostname:'example.test',store_id:'0'},selection:browserSelection};
  const encryptedBootstrap = await encryptRelayPayload(ready.connector_sender_public_key,
    JSON.stringify({binding,bootstrap}),web);
  for (const listener of bridgeListeners) listener({type:'browser_import.bootstrap_envelope.v1',
    ...binding,envelope:encryptedBootstrap.payload});
  await waitFor(() => bridgeMessages.some(value => value.type === 'browser_import.challenge.v1'));
  const challenge = bridgeMessages.find(value => value.type === 'browser_import.challenge.v1').challenge;
  const pairing = {version:1,bootstrap_id:challenge.bootstrap_id,
    enrollment_nonce:challenge.enrollment_nonce,connector_sender_public_key:challenge.connector_sender_public_key,
    expires_at_ms:expiresAtMs,relay_url:'wss://relay.fixture/ws',relay_auth_token:'fixture-token',
    daemon_id:'kernel',kernel_public_key:kernel.publicKeyBase64,protocol_version:321,
    delivery_capability:{name:'browser_import_final_delivery',version:1},
    source:{current_profile:true,store_id:'0'},selection:browserSelection};
  const encryptedPairing = await encryptRelayPayload(ready.connector_sender_public_key,
    JSON.stringify({binding,pairing}),web);
  for (const listener of bridgeListeners) listener({type:'browser_import.pairing_envelope.v1',
    ...binding,envelope:encryptedPairing.payload});
  await waitFor(() => element('confirm').hidden === false);
  element('start').click();
  await waitFor(() => ['Import completed.','No cookies were delivered.','Import could not continue. No cookies were delivered.',
    'This connector needs the production browser-import runtime delivery command.'].includes(
    element('message').textContent)).catch(() => assert.fail(JSON.stringify({message:element('message').textContent,
      permissionMessages,bridge:bridgeMessages.map(value => ({type:value.type,code:value.code})),wire:wire.length,
      metadata:metadata.map(value => Object.keys(value)[0]),privateDelivery:!!privateDelivery})));
  assert.equal(element('message').textContent,'Import completed.',JSON.stringify({
    requests:metadata.map(value => Object.keys(value)[0]),permissionMessages,privateDelivery:!!privateDelivery}));
  assert.equal(wire.some(value => value.includes('fixture-secret')),false);
  assert.equal(JSON.stringify(bridgeMessages).includes('fixture-token'),false);
  assert.equal(JSON.stringify(metadata).includes('fixture-secret'),false);
  assert.equal(JSON.parse(Buffer.from(privateDelivery.payload_base64,'base64'))[0].value,'fixture-secret');
  assert.equal(element('result').children[0].textContent,'example.test: imported');
  assert.deepEqual(permissionMessages,['reserve','activate','release']);
  assert.deepEqual(sourceCookie,{name:'session',value:'fixture-secret',domain:'example.test',path:'/',
    hostOnly:true,secure:true,httpOnly:true,session:true,sameSite:'lax',storeId:'0'});
});

test('packaged confirmation path delivers only an encrypted batch and releases its permission lease', async t => {
  const packaged = await packageTree(t);
  const [{createRelayKeypair,decryptRelayPayload,encryptRelayPayload},
    {connectBrowserImportRelay},{prepareChromeCookieImport},{deliverBrowserImport},wsModule] = await Promise.all([
    load(packaged,'packages/kernel-client/src/browser-relay-crypto.js'),
    load(packaged,'apps/browser-session-import/relay-connector.mjs'),
    load(packaged,'apps/browser-session-import/chrome-import-consent-flow.mjs'),
    load(packaged,'apps/browser-session-import/chrome-extension/delivery-adapter.mjs'),
    import(process.env.WS_MODULE),
  ]);
  const ws = wsModule;
  const kernel = await createRelayKeypair();
  const sender = await createRelayKeypair();
  const server = new ws.WebSocketServer({host:'127.0.0.1',port:0,maxPayload:1048576});
  await once(server,'listening');
  t.after(async () => {
    for (const peer of server.clients) peer.terminate();
    await new Promise(resolve => server.close(resolve));
  });
  const wire = [];
  const metadata = [];
  let privateDelivery;
  server.on('connection',peer => peer.on('message',async data => {
    const text = String(data);
    wire.push(text);
    const frame = JSON.parse(text);
    if (frame.kind === 'client_connect') {
      peer.send(JSON.stringify({kind:'client_connected',target:{daemon_id:'kernel'},
        daemon_public_key:kernel.publicKeyBase64}));
      return;
    }
    const decoded = JSON.parse(await decryptRelayPayload(kernel.privateKey,
      frame.encrypted_request,sender.publicKeyBase64));
    let response;
    if (decoded.browser_import_delivery) {
      privateDelivery = decoded.browser_import_delivery;
      response = {BrowserImportDelivered:{results:[
        {domain:'example.test',status:'imported',cookie_count:1},
      ]}};
    } else {
      metadata.push(decoded.request);
      const kind = Object.keys(decoded.request)[0];
      response = {BrowserImportConsent:{request_id:requestId,status:{
        PrepareBrowserImport:'prepared',ApproveBrowserImport:'approved',
        ClaimBrowserImportSource:'source_claimed',AuthorizeBrowserImportSource:'source_authorized',
        CancelBrowserImport:'cancelled'}[kind]}};
    }
    const encrypted = await encryptRelayPayload(sender.publicKeyBase64,
      JSON.stringify({request_nonce:frame.encrypted_request.nonce,response}),kernel);
    peer.send(JSON.stringify({kind:'client_response',request_id:frame.request_id,
      encrypted_response:encrypted.payload,error:null}));
  }));

  const relay = await connectBrowserImportRelay({relayUrl:`ws://127.0.0.1:${server.address().port}`,
    authToken:'synthetic-token',daemonId:'kernel',kernelPublicKey:kernel.publicKeyBase64,
    sender,protocolVersion:321,WebSocketImpl:ws.WebSocket});
  t.after(() => relay.close());
  const sourceCookie = {name:'session',value:'fixture-secret',domain:'example.test',path:'/',
    hostOnly:true,secure:true,httpOnly:true,session:true,sameSite:'lax',storeId:'normal'};
  const lifecycle = [];
  const chrome = {permissions:{request:() => {lifecycle.push('permission'); return Promise.resolve(true);},
    contains:async () => true},tabs:{get:async () => ({id:7,incognito:false})},
    cookies:{getAllCookieStores:async () => [{id:'normal',tabIds:[7]}],
      getAll:async () => [structuredClone(sourceCookie)]}};
  const permissionLifecycle = {reserve:async () => lifecycle.push('reserve'),
    activate:async () => lifecycle.push('activate'),release:async () => lifecycle.push('release')};
  const flow = await prepareChromeCookieImport({chrome,selection,sourceTabId:7,
    request:relay.request,permissionLifecycle});
  const completion = flow.confirmAndDeliver((value,options) =>
    deliverBrowserImport({deliver:relay.deliver,...value},options));
  assert.deepEqual(lifecycle,['reserve','permission']);
  assert.deepEqual(await completion,{requestId,status:'completed',
    domains:[{domain:'example.test',status:'imported'}]});
  assert.deepEqual(lifecycle,['reserve','permission','activate','release']);
  assert.deepEqual(await flow.cancel(),{kernelCancellationConfirmed:false});
  assert.equal(metadata.some(value => Object.hasOwn(value,'CancelBrowserImport')),false);
  assert.equal(wire.some(value => value.includes('fixture-secret')),false);
  assert.equal(JSON.stringify(metadata).includes('fixture-secret'),false);
  assert.equal(JSON.parse(Buffer.from(privateDelivery.payload_base64,'base64'))[0].value,'fixture-secret');
  assert.deepEqual(sourceCookie,{name:'session',value:'fixture-secret',domain:'example.test',path:'/',
    hostOnly:true,secure:true,httpOnly:true,session:true,sameSite:'lax',storeId:'normal'});
});

test('packaged failed delivery releases once and sends request-scoped cancellation at most once', async t => {
  const packaged = await packageTree(t);
  const {prepareChromeCookieImport} = await load(packaged,
    'apps/browser-session-import/chrome-import-consent-flow.mjs');
  const calls = [];
  const chrome = {permissions:{request:() => Promise.resolve(true),contains:async () => true},
    tabs:{get:async () => ({id:7,incognito:false})},cookies:{
      getAllCookieStores:async () => [{id:'normal',tabIds:[7]}],
      getAll:async () => [{name:'session',value:'fixture-secret',domain:'example.test',path:'/',
        hostOnly:true,secure:true,httpOnly:true,session:true,sameSite:'lax',storeId:'normal'}]}};
  const request = async value => {
    const kind = Object.keys(value)[0];
    calls.push(structuredClone(value));
    return {BrowserImportConsent:{request_id:requestId,status:{PrepareBrowserImport:'prepared',
      ApproveBrowserImport:'approved',ClaimBrowserImportSource:'source_claimed',
      AuthorizeBrowserImportSource:'source_authorized',CancelBrowserImport:'cancelled'}[kind]}};
  };
  let releases = 0;
  const flow = await prepareChromeCookieImport({chrome,selection,sourceTabId:7,request,
    permissionLifecycle:{reserve:async () => {},activate:async () => {},release:async () => {releases++;}}});
  await assert.rejects(flow.confirmAndDeliver(async () => { throw new Error('private-marker'); }),
    {code:'browser_import_delivery_unavailable'});
  await flow.cancel();
  await flow.cancel();
  assert.equal(releases,1);
  const cancellations = calls.filter(value => Object.hasOwn(value,'CancelBrowserImport'));
  assert.deepEqual(cancellations,[{CancelBrowserImport:{session_id:'room',attachment_id:'attachment',
    request_id:requestId}}]);
  assert.equal(JSON.stringify(calls).includes('fixture-secret'),false);
});

async function packageTree(t) {
  assert.ok(process.env.TYPESCRIPT_MODULE);
  assert.ok(process.env.WS_MODULE);
  const root = await mkdtemp(path.join(tmpdir(),'chariox-packaged-delivery-'));
  t.after(() => rm(root,{recursive:true,force:true}));
  await packageChromeExtension(root,(await import(process.env.TYPESCRIPT_MODULE)).default);
  return root;
}

function load(root,relative) { return import(pathToFileURL(path.join(root,relative)).href); }

class FixtureElement {
  value = '';
  textContent = '';
  hidden = true;
  disabled = false;
  children = [];
  #listeners = new Map();
  addEventListener(kind,listener) { this.#listeners.set(kind,listener); }
  click() { this.#listeners.get('click')?.(); }
  append(value) { this.children.push(value); }
  replaceChildren(...values) { this.children = values; }
}

function replaceGlobals(values) {
  const descriptors = new Map();
  for (const [key,value] of Object.entries(values)) {
    descriptors.set(key,Object.getOwnPropertyDescriptor(globalThis,key));
    Object.defineProperty(globalThis,key,{configurable:true,writable:true,value});
  }
  return {restore() {
    for (const [key,descriptor] of descriptors) {
      if (descriptor) Object.defineProperty(globalThis,key,descriptor);
      else delete globalThis[key];
    }
  }};
}

async function waitFor(predicate) {
  const deadline = Date.now() + 3000;
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error('fixture deadline');
    await new Promise(resolve => setTimeout(resolve,5));
  }
}
