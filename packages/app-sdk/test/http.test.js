import { test, mock } from 'node:test';
import assert from 'node:assert/strict';
import { createAppSdk } from '../src/index.js';
import { createHttp } from '../src/http.js';
import { AppError } from '../src/errors.js';
import { fakeTransport, generation, envelope, response, deferred, flush } from './helpers.js';
const ID = '07f5a249-a00b-4196-acdf-938265f88e0a';
const FIXTURE_URL = 'https://api.example.test/upload';
function fixture(handler) {
  const transport = fakeTransport();
  const send = transport.send.bind(transport);
  transport.send = message => {
    send(message);
    if (message.kind !== 'request') return;
    Promise.resolve().then(() => handler(message)).then(value => transport.receive(response(message.id, value)), error => {
      transport.receive(envelope({kind:'response',id:message.id,error:{code:error.code ?? 'FAILED',message:'bounded fixture error',retryable:false}}));
    });
  };
  const sdk = createAppSdk({transport,generation,paths:{package:'/app',data:'/data',temporary:'/tmp'}});
  return {sdk,transport};
}
test('buffered HTTP composes the real peer stream methods and binary chunks without a legacy RPC', async () => {
  const body = Buffer.alloc(130 * 1024, 217);
  const writes = [];
  let heads = 0, reads = 0;
  const {sdk,transport} = fixture(message => {
    switch (message.method) {
      case 'http.open': assert.equal(message.params.hasBody,true); return {streamId:ID};
      case 'http.write': writes.push(message.params); return {bytesWritten:Buffer.from(message.params.bodyBase64,'base64').length};
      case 'http.headers': return heads++ === 0 ? {pending:true} : {pending:false,status:201,headers:[['content-type','application/octet-stream']],url:FIXTURE_URL};
      case 'http.read': return reads++ === 0 ? {pending:false,done:false,chunkBase64:'AP9vaw=='} : {pending:false,done:true,chunkBase64:''};
      case 'http.cancel': return null;
      default: throw new Error('unexpected method');
    }
  });
  try {
    const value = await sdk.http.request({url:FIXTURE_URL,method:'POST',body});
    assert.equal(value.bodyBase64,'AP9vaw==');
    assert.equal(value.status,201);
    assert.deepEqual(writes.map(write => write.end),[false,false,true]);
    assert.deepEqual(Buffer.concat(writes.map(write => Buffer.from(write.bodyBase64,'base64'))),body);
    assert.equal(transport.sent.filter(message => message.method === 'http.cancel').length,1);
    assert(!transport.sent.some(message => message.method === 'http.request'));
  } finally { sdk.close(); }
});
test('one writer and one reader are in flight; a lost body completion is never retried', async () => {
  const write = deferred(), read = deferred();
  const calls = [];
  const http = createHttp((method,params) => {
    calls.push(method);
    return method === 'http.write' ? write.promise : read.promise;
  });
  const first = http.write(ID,'one');
  await assert.rejects(http.write(ID,'two'),{code:'APP_BUSY'});
  const pending = http.read(ID);
  await assert.rejects(http.headers(ID),{code:'APP_BUSY'});
  await flush();
  assert.deepEqual(calls,['http.write','http.read']);
  write.resolve({bytesWritten:3}); read.resolve({pending:true});
  await Promise.all([first,pending]);
  let attempts = 0;
  const broken = createHttp(() => { attempts++; return Promise.reject(new AppError('APP_HTTP_OUTCOME_UNCERTAIN','unknown')); });
  await assert.rejects(broken.write(ID,'effect'),{code:'APP_HTTP_OUTCOME_UNCERTAIN'});
  assert.equal(attempts,1);
});
test('aborting a pending body pull cancels its peer request and the owned stream', async () => {
  const pulling = deferred(), never = deferred();
  const {sdk,transport} = fixture(message => {
    if (message.method === 'http.open') return {streamId:ID};
    if (message.method === 'http.headers') return {pending:false,status:200,headers:[],url:FIXTURE_URL};
    if (message.method === 'http.read') { pulling.resolve(); return never.promise; }
    if (message.method === 'http.cancel') return null;
    throw new Error('unexpected');
  });
  const controller = new AbortController();
  try {
    const pending = sdk.http.request({url:FIXTURE_URL},{signal:controller.signal});
    await pulling.promise;
    controller.abort();
    await assert.rejects(pending,{code:'CANCELLED'});
    assert.equal(transport.sent.filter(message => message.method === 'http.read').length,1);
    assert.equal(transport.sent.filter(message => message.kind === 'cancel').length,1);
    assert.equal(transport.sent.filter(message => message.method === 'http.cancel').length,1);
    never.resolve({pending:false,done:false,chunkBase64:'eA=='});
    await flush();
  } finally { sdk.close(); }
});
test('buffered response caps aggregate bytes and preserves the original deadline across pulls', async () => {
  let time = 1000, reads = 0;
  const elapsed = [];
  const clock = mock.method(performance,'now',()=>time);
  const http = createHttp(async (method,params,options) => {
    if (method === 'http.cancel') { assert.equal(options.signal,undefined); return null; }
    elapsed.push(options.timeoutMs);
    time += 10;
    if (method === 'http.open') return {streamId:ID};
    if (method === 'http.headers') return {pending:false,status:200,headers:[],url:FIXTURE_URL};
    reads++;
    return {pending:false,done:false,chunkBase64:Buffer.alloc(64*1024).toString('base64')};
  });
  try {
    await assert.rejects(http.request({url:FIXTURE_URL},{timeoutMs:1000}),{code:'APP_HTTP_LIMIT'});
    assert.equal(reads,9);
    assert.deepEqual(elapsed,Array.from({length:11},(_,n)=>1000-n*10));
  } finally { clock.mock.restore(); }
});
test('SDK bounds chunks, fields and HTTPS before sending; final empty chunk is allowed', async () => {
  const calls = [];
  const http = createHttp((method,params)=> { calls.push({method,params}); return Promise.resolve({bytesWritten:0}); });
  for (const request of [{url:'http://example.test'}, {url:FIXTURE_URL,owner:'alice'}, {url:FIXTURE_URL,connectionId:null}, {url:FIXTURE_URL,method:'GET',hasBody:true}]) {
    assert.throws(()=>http.open(request),{code:'INVALID_ARGUMENT'});
  }
  assert.throws(()=>http.write(ID,new Uint8Array(64*1024+1)),{code:'INVALID_ARGUMENT'});
  assert.throws(()=>http.write(ID,'',false),{code:'INVALID_ARGUMENT'});
  assert.throws(()=>http.read('../foreign'),{code:'INVALID_ARGUMENT'});
  assert.equal(calls.length,0);
  await http.write(ID,'',true);
  assert.deepEqual(calls,[{method:'http.write',params:{streamId:ID,bodyBase64:'',end:true}}]);
});
test('shared versioned HTTP wire fixture matches the SDK operations exactly', async () => {
  const {readFile} = await import('node:fs/promises');
  const contract = JSON.parse(await readFile(new URL('./http-contract.json',import.meta.url),'utf8'));
  const metadata = JSON.parse(await readFile(new URL('../package.json',import.meta.url),'utf8'));
  assert.equal(contract.sdkVersion,metadata.version);
  for (const entry of contract.cases) {
    const http = createHttp((method,params)=> {
      assert.equal(method,entry.method); assert.deepEqual(params,entry.params);
      return Promise.resolve(entry.result);
    });
    let result;
    if (entry.method === 'http.open') result = await http.open(entry.params);
    else if (entry.method === 'http.write') result = await http.write(entry.params.streamId,Buffer.from(entry.params.bodyBase64,'base64'),entry.params.end);
    else result = await http[entry.method.slice(5)](entry.params.streamId);
    assert.deepEqual(result,entry.result);
  }
});
