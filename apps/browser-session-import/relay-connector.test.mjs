import assert from 'node:assert/strict';
import {EventEmitter, once} from 'node:events';
import test from 'node:test';
import {createRelayKeypair, decryptRelayPayload, encryptRelayPayload}
  from '../../packages/kernel-client/src/browser-relay-crypto.ts';
import {authorizeBrowserImportSourceRequest,prepareBrowserImportRequest,approveBrowserImportRequest,
  cancelBrowserImportRequest,claimBrowserImportSourceRequest}
  from '../../packages/kernel-client/src/browser-import-requests.ts';
import {connectBrowserImportRelay} from './relay-connector.mjs';
import {readKernelApprovedChromeCookies} from './kernel-source-reader.mjs';

const {WebSocketServer} = await import(process.env.WS_MODULE ?? 'ws');
const selection = {session_id:'room-1',attachment_id:'attachment-1',environment_id:'environment-1',
  runtime_generation:1,tab_id:'tab-1',document_revision:1,source_store_id:'normal',
  domains:['example.test'],partition_sites:[],overwrite:false};

test('paired import requests cross a live socket using the existing encrypted relay frames', async () => {
  const kernel = await createRelayKeypair();
  const sender = await createRelayKeypair();
  const server = new WebSocketServer({host:'127.0.0.1',port:0,maxPayload:262144});
  await once(server,'listening');
  const observed = [];
  const failures = [];
  server.on('connection', socket => socket.on('message', async data => {
    try {
      const frame = JSON.parse(String(data));
      if (frame.kind === 'client_connect') {
        assert.equal(frame.auth_token,'synthetic-relay-token');
        socket.send(JSON.stringify({kind:'client_connected',target:{daemon_id:'kernel-1'},
          daemon_public_key:kernel.publicKeyBase64}));
        return;
      }
      assert.equal(frame.kind,'client_request');
      assert.deepEqual(frame.target,{daemon_id:'kernel-1'});
      assert.equal(frame.encrypted_request.sender_public_key,sender.publicKeyBase64);
      assert.equal(String(data).includes('example.test'),false);
      const decoded = JSON.parse(await decryptRelayPayload(kernel.privateKey,frame.encrypted_request,sender.publicKeyBase64));
      observed.push(decoded);
      const response = {BrowserImportConsent:{request_id:'a'.repeat(32),status:'source_authorized'}};
      const encrypted = await encryptRelayPayload(sender.publicKeyBase64,
        JSON.stringify({request_nonce:frame.encrypted_request.nonce,response}),kernel);
      socket.send(JSON.stringify({kind:'client_response',request_id:frame.request_id,
        encrypted_response:encrypted.payload,error:null}));
    } catch (error) { failures.push(error); socket.close(); }
  }));
  let client;
  try {
    client = await connectBrowserImportRelay({relayUrl:`ws://127.0.0.1:${server.address().port}`,
      authToken:'synthetic-relay-token',daemonId:'kernel-1',kernelPublicKey:kernel.publicKeyBase64,
      sender,protocolVersion:317});
    const request = authorizeBrowserImportSourceRequest('a'.repeat(32),selection);
    const result = await client.request(request);
    assert.deepEqual(result,{BrowserImportConsent:{request_id:'a'.repeat(32),status:'source_authorized'}});
    assert.deepEqual(observed[0].request,request);
    assert.equal(typeof observed[0].command_id,'string');
    assert.deepEqual(failures,[]);
  } finally {
    client?.close();
    for (const socket of server.clients) socket.terminate();
    await new Promise(resolve => server.close(resolve));
  }
});

// Only the socket peer is a fixture. Framing, encryption, reply validation and
// source-reader composition execute the production modules.
async function fixture(t, {handshake, onRequest} = {}) {
  const kernel = await createRelayKeypair();
  const sender = await createRelayKeypair();
  const server = new WebSocketServer({host:'127.0.0.1',port:0,maxPayload:262144});
  await once(server,'listening');
  const received = new EventEmitter();
  const requests = [];
  const failures = [];
  const clients = [];
  const wire = [];
  const peers = [];
  t.after(async () => {
    for (const client of clients) client.close();
    for (const peer of server.clients) peer.terminate();
    await new Promise(resolve => server.close(resolve));
    assert.deepEqual(failures,[]);
  });
  async function reply(record, {nonce = record.frame.encrypted_request.nonce,
    key = kernel, id = record.frame.request_id, requestId = 'a'.repeat(32),
    status = 'source_authorized', response = {BrowserImportConsent:{request_id:requestId,status}}} = {}) {
    const encrypted = await encryptRelayPayload(sender.publicKeyBase64,
      JSON.stringify({request_nonce:nonce,response}),key);
    record.peer.send(JSON.stringify({kind:'client_response',request_id:id,
      encrypted_response:encrypted.payload,error:null}));
  }
  server.on('connection', peer => {
    peers.push(peer);
    peer.on('message', async data => {
      try {
        wire.push(String(data));
        const frame = JSON.parse(String(data));
        if (frame.kind === 'client_connect') {
          const connected = {kind:'client_connected',target:{daemon_id:'kernel-1'},
            daemon_public_key:kernel.publicKeyBase64};
          peer.send(JSON.stringify(handshake ? handshake(connected) : connected));
          return;
        }
        const request = JSON.parse(await decryptRelayPayload(kernel.privateKey,
          frame.encrypted_request,sender.publicKeyBase64)).request;
        const record = {frame,request,peer};
        requests.push(record);
        received.emit('request');
        if (onRequest) await onRequest(record,reply);
      } catch (error) { failures.push(error); peer.close(); }
    });
  });
  const options = {relayUrl:`ws://127.0.0.1:${server.address().port}`,
    authToken:'synthetic-relay-token',daemonId:'kernel-1',kernelPublicKey:kernel.publicKeyBase64,
    sender,protocolVersion:317,timeoutMs:1000};
  return {options,requests,reply,wire,peers,
    async connect(overrides = {}) {
      const client = await connectBrowserImportRelay({...options,...overrides});
      clients.push(client);
      return client;
    },
    async count(expected) {
      while (requests.length < expected) await once(received,'request', {signal:AbortSignal.timeout(2000)});
    }};
}

const authorize = () => authorizeBrowserImportSourceRequest('a'.repeat(32),selection);
const denied = {message:'browser import transport unavailable'};

test('all five consent operations preserve their shared request and response contracts', async t => {
  const cases = [[prepareBrowserImportRequest(selection),'prepared'],
    [approveBrowserImportRequest('a'.repeat(32),selection),'approved'],
    [claimBrowserImportSourceRequest('a'.repeat(32),selection),'source_claimed'],
    [authorize(),'source_authorized'],
    [cancelBrowserImportRequest('room-1','attachment-1','a'.repeat(32)),'cancelled']];
  let index = 0;
  const f = await fixture(t,{onRequest:(record,reply) => reply(record,{status:cases[index++][1]})});
  const client = await f.connect();
  for (const [request,status] of cases) {
    assert.deepEqual(await client.request(request),{BrowserImportConsent:{request_id:'a'.repeat(32),status}});
  }
  assert.deepEqual(f.requests.map(record => record.request),cases.map(([request]) => request));
});

test('wrong handshake kernel or target never receives an import request', async t => {
  for (const field of ['key','target']) {
    const f = await fixture(t,{handshake:frame => field === 'key'
      ? {...frame,daemon_public_key:'untrusted-key'} : {...frame,target:{daemon_id:'other-kernel'}}});
    await assert.rejects(f.connect(),denied);
    assert.equal(f.requests.length,0);
  }
});

test('unsafe URLs, unsupported protocol and invalid signals never open a socket', async t => {
  const f = await fixture(t);
  let opened = 0;
  class ForbiddenSocket { constructor() { opened++; throw new Error('private-marker'); } }
  for (const overrides of [{relayUrl:'ws://remote.example.test'},
    {relayUrl:'wss://relay.example.test/?token=private-marker'},
    {relayUrl:'wss://user:private-marker@relay.example.test/'},
    {relayUrl:'https://relay.example.test'}, {protocolVersion:316},
    {signal:{}}, {signal:AbortSignal.abort()}, {timeoutMs:Infinity}]) {
    await assert.rejects(f.connect({...overrides,WebSocketImpl:ForbiddenSocket}),denied);
  }
  assert.equal(opened,0);
});

test('forged, replayed, wrong-phase and wrong-consent responses are rejected', async t => {
  const attacker = await createRelayKeypair();
  for (const overrides of [{nonce:'AAAAAAAAAAAAAAAA'}, {key:attacker},
    {status:'approved'}, {requestId:'b'.repeat(32)}, {response:true},
    {response:{BrowserImportConsent:{request_id:'a'.repeat(32),status:'source_authorized',cookies:[]}}}]) {
    const f = await fixture(t,{onRequest:(record,reply) => reply(record,overrides)});
    const client = await f.connect();
    await assert.rejects(client.request(authorize()),denied);
  }
});

test('an old encrypted response cannot authorize a new request even with its outer ID', async t => {
  const f = await fixture(t);
  const client = await f.connect();
  const first = client.request(authorize());
  await f.count(1);
  await f.reply(f.requests[0]);
  await first;
  const second = assert.rejects(client.request(authorize()),denied);
  await f.count(2);
  await f.reply(f.requests[1],{nonce:f.requests[0].frame.encrypted_request.nonce});
  await second;
});

test('timeouts, cancellation and close settle requests; late replies cannot revive them', async t => {
  for (const mode of ['timeout','cancel','close','disconnect','lifetime-abort']) {
    const f = await fixture(t);
    const lifetime = new AbortController();
    const client = await f.connect({signal:lifetime.signal});
    const controller = new AbortController();
    const pending = assert.rejects(client.request(authorize(),
      {signal:controller.signal,timeoutMs:mode === 'timeout' ? 80 : 1000}),denied);
    await f.count(1);
    if (mode === 'cancel') controller.abort('private-marker');
    if (mode === 'close') client.close();
    if (mode === 'disconnect') f.requests[0].peer.close();
    if (mode === 'lifetime-abort') lifetime.abort();
    await pending;
    if (mode === 'timeout' || mode === 'cancel') {
      await f.reply(f.requests[0]);
      const next = client.request(authorize());
      await f.count(2);
      await f.reply(f.requests[1]);
      await next;
    } else await assert.rejects(client.request(authorize()),denied);
  }
});

test('eight pending requests bound resource use and cancellation frees a slot', async t => {
  const f = await fixture(t);
  const client = await f.connect();
  const controllers = Array.from({length:8},() => new AbortController());
  const pending = controllers.map(controller => assert.rejects(
    client.request(authorize(),{signal:controller.signal}),denied));
  await f.count(8);
  await assert.rejects(client.request(authorize()),denied);
  controllers[0].abort();
  await pending[0];
  const replacement = client.request(authorize());
  await f.count(9);
  await f.reply(f.requests[8]);
  await replacement;
  client.close();
  await Promise.all(pending);
});

test('metadata and sender identity are snapshotted and cookies never enter requests', async t => {
  const f = await fixture(t,{onRequest:(record,reply) => reply(record)});
  const mutableSender = {...f.options.sender};
  const client = await f.connect({sender:mutableSender});
  mutableSender.publicKeyBase64 = 'replaced-key';
  mutableSender.privateKey = null;
  const request = authorize();
  request.AuthorizeBrowserImportSource.selection.cookies = [{value:'private-marker'}];
  const pending = client.request(request);
  request.AuthorizeBrowserImportSource.selection.domains.push('unapproved.test');
  request.AuthorizeBrowserImportSource.selection.overwrite = true;
  await pending;
  assert.deepEqual(f.requests[0].request,authorize());
  assert.equal(f.wire.join('').includes('private-marker'),false);
  for (const value of [{RunCommand:{command:'private-marker'}},
    {AuthorizeBrowserImportSource:{request_id:'bad',selection}},
    {AuthorizeBrowserImportSource:{request_id:'a'.repeat(32),selection:{...selection,domains:[]}}}]) {
    await assert.rejects(client.request(value),denied);
  }
  await assert.rejects(client.request(authorize(),{signal:{}}),denied);
  assert.equal(f.requests.length,1);
});

test('malformed and oversized frames close the connection and fail pending work', async t => {
  for (const payload of ['{','x'.repeat(262145),JSON.stringify({kind:'unexpected',secret:'private-marker'})]) {
    const f = await fixture(t,{onRequest:record => record.peer.send(payload)});
    const client = await f.connect();
    await assert.rejects(client.request(authorize()),denied);
    await assert.rejects(client.request(authorize()),denied);
  }
});

test('source reader uses live encrypted claims and discards cookies on kernel revocation', async t => {
  for (const revoke of [false,true]) {
    let reads = 0;
    const f = await fixture(t,{onRequest:(record,reply) => reply(record,{status:reads && revoke
      ? 'cancelled' : record.request.ClaimBrowserImportSource ? 'source_claimed' : 'source_authorized'})});
    const client = await f.connect();
    const chrome = {
      tabs:{get:async () => ({id:7,incognito:false})},permissions:{contains:async () => true},
      cookies:{getAllCookieStores:async () => [{id:'normal',tabIds:[7]}],getAll:async () => {
        reads++;
        return [{name:'session',value:'fixture-secret',domain:'example.test',path:'/',
          secure:true,httpOnly:true,hostOnly:true,session:true,sameSite:'lax',storeId:'normal'}];
      }},
    };
    const pending = readKernelApprovedChromeCookies({chrome,requestId:'a'.repeat(32),selection,
      sourceTabId:7,request:client.request});
    if (revoke) await assert.rejects(pending,{code:'cookie_source_unavailable'});
    else assert.equal((await pending).cookies[0].value,'fixture-secret');
    assert.equal(reads,1);
    assert.equal(JSON.stringify(f.requests.map(record => record.request)).includes('fixture-secret'),false);
    assert.equal(f.wire.join('').includes('fixture-secret'),false);
  }
});

test('unanswered handshake has a deadline and lifetime abort closes its owned socket', async t => {
  const f = await fixture(t);
  for (const abort of [false,true]) {
    let closed = 0;
    class SilentSocket extends EventTarget {
      close() { closed++; }
    }
    const controller = new AbortController();
    const pending = assert.rejects(f.connect({WebSocketImpl:SilentSocket,
      timeoutMs:20,signal:controller.signal}),denied);
    if (abort) controller.abort();
    await pending;
    assert.equal(closed,1);
  }
});

test('abort before encryption completes prevents dispatch and backpressure rejects new sends', async t => {
  const f = await fixture(t);
  const client = await f.connect();
  const controller = new AbortController();
  const pending = assert.rejects(client.request(authorize(),{signal:controller.signal}),denied);
  controller.abort();
  await pending;
  const next = client.request(authorize());
  await f.count(1);
  await f.reply(f.requests[0]);
  await next;
  assert.equal(f.requests.length,1);

  const sent = [];
  class BackpressuredSocket extends EventTarget {
    bufferedAmount = 262145;
    constructor() {
      super();
      queueMicrotask(() => this.dispatchEvent(new Event('open')));
    }
    send(data) {
      sent.push(JSON.parse(data));
      queueMicrotask(() => this.dispatchEvent(new MessageEvent('message',{data:JSON.stringify({
        kind:'client_connected',target:{daemon_id:'kernel-1'},daemon_public_key:f.options.kernelPublicKey,
      })})));
    }
    close() {}
  }
  const blocked = await f.connect({WebSocketImpl:BackpressuredSocket});
  await assert.rejects(blocked.request(authorize()),denied);
  assert.deepEqual(sent.map(frame => frame.kind),['client_connect']);
});

test('ending a source read releases its pending relay request immediately', async t => {
  for (const mode of ['cancel','deadline']) {
    const f = await fixture(t);
    const client = await f.connect();
    const controller = new AbortController();
    let transportEnded = false;
    const request = (payload,options) => client.request(payload,options)
      .finally(() => {transportEnded = true;});
    const chrome = {
      tabs:{get:async () => ({id:7,incognito:false})},permissions:{contains:async () => true},
      cookies:{getAllCookieStores:async () => [{id:'normal',tabIds:[7]}],getAll:async () => {
        assert.fail('a pending claim must not reach the cookie store');
      }},
    };
    const reading = assert.rejects(readKernelApprovedChromeCookies({chrome,requestId:'a'.repeat(32),
      selection,sourceTabId:7,request,signal:controller.signal,timeoutMs:mode === 'deadline' ? 60 : 1000}),
    {code:mode === 'deadline' ? 'cookie_source_timeout' : 'cookie_source_cancelled'});
    await f.count(1);
    if (mode === 'cancel') controller.abort('private-marker');
    await reading;
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(transportEnded,true,mode);
  }
});
