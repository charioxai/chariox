import assert from 'node:assert/strict';
import test from 'node:test';

import {BrowserImportExternalPortBroker} from './external-port-broker.mjs';

test('broker accepts only exact production origins and binds one connector session',() => {
  const broker = new BrowserImportExternalPortBroker({extensionId:'abcdefghijklmnopabcdefghijklmnop'});
  const connector = port({url:'chrome-extension://abcdefghijklmnopabcdefghijklmnop/apps/browser-session-import/chrome-extension/connector.html',tabId:11,name:'browser-import-web-session-v1',id:'abcdefghijklmnopabcdefghijklmnop'});
  broker.connectConnector(connector);
  connector.emit({type:'browser_import.connector_ready.v1',connector_session_id:'c'.repeat(32),
    connector_sender_public_key:key(),source:{current_profile:true,hostname:'example.test',store_id:'0'}});

  const evil = port({url:'https://lookalike.chariox.com/view',tabId:12});
  broker.connectExternal(evil);
  assert.equal(evil.disconnected,true);
  for (const hostile of [
    port({url:'https://chariox.com/view',origin:'https://staging.chariox.com',tabId:12}),
    port({url:'https://chariox.com/view',tabId:12,id:'ponmlkjihgfedcbaponmlkjihgfedcba'}),
    port({url:'https://chariox.com/view',tabId:12,incognito:true}),
  ]) {
    broker.connectExternal(hostile);
    assert.equal(hostile.disconnected,true);
  }

  const web = port({url:'https://chariox.com/view',tabId:13});
  broker.connectExternal(web);
  web.emit({type:'browser_import.discover.v1',request_id:'a'.repeat(32),
    operation_nonce:'b'.repeat(32),expected_connector_session_id:null});
  assert.equal(web.sent[0].type,'browser_import.discovery.v1');
  assert.equal(web.sent[0].extension_id,'abcdefghijklmnopabcdefghijklmnop');
  assert.equal(web.sent[0].connector_session_id,'c'.repeat(32));
  web.disconnectExternally();
  const operation = port({url:'https://chariox.com/view',tabId:14});
  broker.connectExternal(operation);
  operation.emit({type:'browser_import.discover.v1',request_id:'d'.repeat(32),
    operation_nonce:'e'.repeat(32),expected_connector_session_id:'c'.repeat(32)});
  assert.equal(operation.sent[0].type,'browser_import.discovery.v1');
});

test('broker forwards only bound messages and sends one cancel on disconnect',() => {
  const broker = new BrowserImportExternalPortBroker({extensionId:'abcdefghijklmnopabcdefghijklmnop'});
  const connector = port({url:'chrome-extension://abcdefghijklmnopabcdefghijklmnop/apps/browser-session-import/chrome-extension/connector.html',tabId:11,name:'browser-import-web-session-v1',id:'abcdefghijklmnopabcdefghijklmnop'});
  broker.connectConnector(connector);
  connector.emit({type:'browser_import.connector_ready.v1',connector_session_id:'c'.repeat(32),
    connector_sender_public_key:key(),source:{current_profile:true,hostname:'example.test',store_id:'0'}});
  const web = port({url:'https://staging.chariox.com/view',tabId:13});
  broker.connectExternal(web);
  web.emit({type:'browser_import.discover.v1',request_id:'a'.repeat(32),
    operation_nonce:'b'.repeat(32),expected_connector_session_id:'c'.repeat(32)});
  web.emit({type:'browser_import.bootstrap_envelope.v1',request_id:'a'.repeat(32),
    operation_nonce:'b'.repeat(32),connector_session_id:'c'.repeat(32),target_binding:'binding',
    session_id:'room',daemon_id:'daemon',kernel_public_key:key(),expires_at_ms:Date.now()+10_000,
    envelope:{sender_public_key:key(),nonce:'A'.repeat(16),ciphertext:'cipher'}});
  assert.equal(connector.sent.at(-1).type,'browser_import.bootstrap_envelope.v1');
  web.disconnectExternally();
  web.disconnectExternally();
  assert.equal(connector.sent.filter(value => value.type === 'browser_import.cancel.v1').length,1);
});

test('broker rejects malformed discovery and emits fixed metadata-only failure on connector disconnect',() => {
  const broker = new BrowserImportExternalPortBroker({extensionId:'abcdefghijklmnopabcdefghijklmnop'});
  const connector = port({url:'chrome-extension://abcdefghijklmnopabcdefghijklmnop/apps/browser-session-import/chrome-extension/connector.html',tabId:11,name:'browser-import-web-session-v1',id:'abcdefghijklmnopabcdefghijklmnop'});
  broker.connectConnector(connector);
  connector.emit({type:'browser_import.connector_ready.v1',connector_session_id:'c'.repeat(32),
    connector_sender_public_key:key(),source:{current_profile:true,hostname:'example.test',store_id:'0'}});
  const malformed = port({url:'https://chariox.com/view',tabId:13});
  broker.connectExternal(malformed);
  malformed.emit({type:'browser_import.discover.v1',request_id:'raw-secret',operation_nonce:'b'.repeat(32),expected_connector_session_id:null});
  assert.equal(malformed.disconnected,true);

  const web = port({url:'https://chariox.com/view',tabId:14});
  broker.connectExternal(web);
  web.emit({type:'browser_import.discover.v1',request_id:'a'.repeat(32),operation_nonce:'b'.repeat(32),expected_connector_session_id:null});
  connector.disconnectExternally();
  assert.deepEqual(web.sent.at(-1),{type:'browser_import.failure.v1',request_id:'a'.repeat(32),
    operation_nonce:'b'.repeat(32),connector_session_id:'c'.repeat(32),code:'connector_unavailable'});
  assert.equal(web.disconnected,true);
});

function port({url,tabId,name='chariox-browser-import-v1',id,origin=new URL(url).origin,incognito=false}) {
  const messages=new Set(),disconnects=new Set();
  return {name,sender:{url,origin,id,
      tab:{id:tabId,incognito}},sent:[],disconnected:false,
    onMessage:{addListener:fn=>messages.add(fn),removeListener:fn=>messages.delete(fn)},
    onDisconnect:{addListener:fn=>disconnects.add(fn),removeListener:fn=>disconnects.delete(fn)},
    postMessage(value){this.sent.push(value);},disconnect(){this.disconnected=true;},
    emit(value){for(const fn of [...messages])fn(value);},
    disconnectExternally(){for(const fn of [...disconnects])fn();}};
}
function key(){return 'BBDjFsEd2B4znY9eLUPJWBfJLwJ31gNDNoJ2ofW5ZTscmHFcHjQKkSSCpTDKaXUBf0Q1JvGgwG3PtOfdFWspfxA=';}
