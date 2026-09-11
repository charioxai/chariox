import assert from 'node:assert/strict';
import {randomUUID} from 'node:crypto';
import {mkdtemp,readFile,rm,stat} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import test from 'node:test';

import {applyProductionBrowserImport,recoverProductionBrowserImport} from './production-destination.mjs';

test('protocol-321 production path applies exact domain results and restart cleanup is matching and durable',async () => {
  const [localProtocol,peerProtocol] = await Promise.all([
    readFile(new URL('../kernel/src/local/api/types.rs',import.meta.url),'utf8'),
    readFile(new URL('../kernel/src/transport/relay_peer.rs',import.meta.url),'utf8'),
  ]);
  assert.match(localProtocol,/LOCAL_DAEMON_PROTOCOL_VERSION: u32 = 321/);
  assert.match(peerProtocol,/RELAY_PEER_PROTOCOL_VERSION: u32 = 49/);
  const home = await mkdtemp(path.join(tmpdir(),'chariox-browser-import-production-'));
  const generatedValue = randomUUID();
  let fenced = 0;
  let cookies = [];
  let failVerification = false;
  const connection = {isOpen:() => true,send:async (method,params) => {
    if (method === 'Page.getFrameTree') return {frameTree:{frame:{loaderId:'document'}}};
    if (method === 'Target.getTargetInfo') return {targetInfo:{targetId:'target'}};
    if (method === 'Storage.getCookies') {
      if (failVerification && cookies.length) { failVerification=false; return {cookies:[]}; }
      return {cookies:structuredClone(cookies)};
    }
    if (method === 'Storage.setCookies') {
      cookies = params.cookies.map(({url,...cookie}) => ({...cookie,
        domain:cookie.domain ?? new URL(url).hostname,session:cookie.expires === undefined,
        expires:cookie.expires ?? -1}));
      return {};
    }
    if (method === 'Network.deleteCookies') { cookies=[]; return {}; }
    throw new Error('unexpected fixture command');
  }};
  const controller = {browserGeneration:1,resolvePageTarget:async () => ({connection,sessionId:'cdp'}),
    withCookieWritersQuiesced:async operation => { fenced++; return operation({retain:()=>{}}); }};
  const payload = [{name:'session',value:generatedValue,domain:'example.test',path:'/',secure:true,
    httpOnly:true,hostOnly:true,session:true,sameSite:'lax',storeId:'normal'}];
  try {
    controller.browserGeneration=2;
    assert.deepEqual(await applyProductionBrowserImport({controller,home,params:{
      binding:{request_id:'f'.repeat(32),user_id:'user',room_id:'room',environment_id:'environment'},
      browser_generation:1,target_id:'target',document_id:'document',source_store_id:'normal',
      domains:['example.test'],partition_sites:[],overwrite:false,payload_json:JSON.stringify(payload),
    }}),{status:'rolled_back',results:[]});
    assert.deepEqual(cookies,[]);
    controller.browserGeneration=1;
    const result = await applyProductionBrowserImport({controller,home,params:{
      binding:{request_id:'a'.repeat(32),user_id:'user',room_id:'room',environment_id:'environment'},
      browser_generation:1,target_id:'target',document_id:'document',source_store_id:'normal',
      domains:['example.test','empty.test'],partition_sites:[],overwrite:false,payload_json:JSON.stringify(payload),
    }});
    assert.deepEqual(result,{status:'applied',results:[
      {domain:'example.test',status:'imported',cookie_count:1},
      {domain:'empty.test',status:'no_cookies',cookie_count:0},
    ]});
    assert.equal(fenced,2);
    assert.equal(cookies[0].value,generatedValue);
    const directory = path.join(home,'browser-import',
      'ba5285161ba6eed0085fb13784ce5c92f70ebc268b94fd66aa1d68a32884204d');
    assert.equal((await stat(path.join(directory,'journal.key'))).mode & 0o077,0);
    const outcome = await readFile(path.join(directory,'cookie-import.completed'),'utf8');
    assert.equal(outcome.includes(generatedValue),false);
    await assert.rejects(stat(path.join(directory,'cookie-import.pending')),{code:'ENOENT'});
    assert.deepEqual(await recoverProductionBrowserImport({controller,home,params:{
      binding:{request_id:'a'.repeat(32),user_id:'user',room_id:'room',environment_id:'environment'},
      target_id:'target',
    }}),{status:'verified'});
    await assert.rejects(recoverProductionBrowserImport({controller,home,params:{
      binding:{request_id:'c'.repeat(32),user_id:'user',room_id:'room',environment_id:'environment'},
      target_id:'target',
    }}),/cookie_import_recovery_required/);

    cookies=[];
    failVerification=true;
    const rolledBack = await applyProductionBrowserImport({controller,home,params:{
      binding:{request_id:'b'.repeat(32),user_id:'user',room_id:'room',environment_id:'environment'},
      browser_generation:1,target_id:'target',document_id:'document',source_store_id:'normal',
      domains:['example.test'],partition_sites:[],overwrite:false,payload_json:JSON.stringify(payload),
    }});
    assert.deepEqual(rolledBack,{status:'rolled_back',results:[]});
    assert.deepEqual(cookies,[]);
    await assert.rejects(stat(path.join(directory,'cookie-import.pending')),{code:'ENOENT'});
  } finally { await rm(home,{recursive:true,force:true}); }
});
