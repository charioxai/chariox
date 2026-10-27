export const browserImportWebOrigins = Object.freeze(['https://chariox.com','https://staging.chariox.com']);
const externalName = 'chariox-browser-import-v1';
const connectorName = 'browser-import-web-session-v1';
const maximumSessions = 8;

export class BrowserImportExternalPortBroker {
  constructor({extensionId}) {
    if (!extensionIdValue(extensionId)) throw new Error('browser import extension identity unavailable');
    this.extensionId=extensionId;
    this.sessions=new Map();
  }

  connectConnector(port) {
    if (!validConnectorPort(port,this.extensionId)) { disconnect(port); return; }
    let registered=null;
    const onMessage=value=>{
      if (registered) { this.forwardConnector(registered,value); return; }
      const registration=connectorRegistration(value);
      if (!registration || this.sessions.size >= maximumSessions || this.sessions.has(registration.id)) {
        disconnect(port); return;
      }
      registered={...registration,port,web:null,binding:null,cancelled:false,active:false};
      this.sessions.set(registration.id,registered);
    };
    const onDisconnect=()=>{
      if (!registered) return;
      this.sessions.delete(registered.id);
      if (registered.web) { safePost(registered.web,{type:'browser_import.failure.v1',
        ...registered.binding,code:'connector_unavailable'}); disconnect(registered.web); }
    };
    port.onMessage.addListener(onMessage); port.onDisconnect.addListener(onDisconnect);
  }

  connectExternal(port) {
    if (!validWebPort(port)) { disconnect(port); return; }
    let session=null;
    const onMessage=value=>{
      if (!session) {
        const discover=discoveryRequest(value);
        if (!discover) { disconnect(port); return; }
        const candidates=[...this.sessions.values()].filter(item=>!item.web
          && (discover.expected===null || item.id===discover.expected));
        if (candidates.length !== 1) { disconnect(port); return; }
        session=candidates[0]; session.web=port;
        session.cancelled=false; session.active=false;
        session.binding={request_id:discover.requestId,operation_nonce:discover.nonce,
          connector_session_id:session.id};
        safePost(port,{type:'browser_import.discovery.v1',...session.binding,
          extension_id:this.extensionId,connector_sender_public_key:session.senderPublicKey,
          source:session.source});
        return;
      }
      if (!boundWebMessage(value,session.binding)) { this.cancelSession(session); disconnect(port); return; }
      if (value.type === 'browser_import.bootstrap_envelope.v1') session.active=true;
      safePost(session.port,value);
    };
    const onDisconnect=()=>{ if (session) this.cancelSession(session); };
    port.onMessage.addListener(onMessage); port.onDisconnect.addListener(onDisconnect);
  }

  forwardConnector(session,value) {
    if (!session.web || !boundConnectorMessage(value,session.binding)) return;
    safePost(session.web,value);
    if (value.type === 'browser_import.result.v1' || value.type === 'browser_import.failure.v1') {
      const web=session.web;
      session.web=null; session.binding=null; session.active=false;
      disconnect(web);
    }
  }

  cancelSession(session) {
    const binding=session.binding;
    const shouldCancel=Boolean(binding && session.active && !session.cancelled);
    session.cancelled=shouldCancel;
    session.web=null; session.binding=null; session.active=false;
    if (shouldCancel) safePost(session.port,{type:'browser_import.cancel.v1',...binding});
  }
}

export function installBrowserImportExternalPortBroker(chrome) {
  const broker=new BrowserImportExternalPortBroker({extensionId:chrome.runtime.id});
  chrome.runtime.onConnect.addListener(port=>{ if (port.name===connectorName) broker.connectConnector(port); });
  chrome.runtime.onConnectExternal.addListener(port=>{ if (port.name===externalName) broker.connectExternal(port); });
  return broker;
}

function connectorRegistration(value) {
  if (!plain(value) || !exact(value,['type','connector_session_id','connector_sender_public_key','source'])
      || value.type!=='browser_import.connector_ready.v1' || !identifier(value.connector_session_id)
      || !relayKey(value.connector_sender_public_key) || !source(value.source)) return null;
  return {id:value.connector_session_id,senderPublicKey:value.connector_sender_public_key,
    source:Object.freeze({...value.source})};
}
function discoveryRequest(value) { if (!plain(value)
    || !exact(value,['type','request_id','operation_nonce','expected_connector_session_id'])
    || value.type!=='browser_import.discover.v1' || !identifier(value.request_id)
    || !identifier(value.operation_nonce) || !(value.expected_connector_session_id===null
      || identifier(value.expected_connector_session_id))) return null;
  return {requestId:value.request_id,nonce:value.operation_nonce,expected:value.expected_connector_session_id}; }
function boundWebMessage(value,binding) { return plain(value)
  && ['browser_import.bootstrap_envelope.v1','browser_import.pairing_envelope.v1','browser_import.cancel.v1'].includes(value.type)
  && value.request_id===binding.request_id && value.operation_nonce===binding.operation_nonce
  && value.connector_session_id===binding.connector_session_id && serialized(value)<=131072; }
function boundConnectorMessage(value,binding) { return plain(value)
  && ['browser_import.challenge.v1','browser_import.ready.v1','browser_import.progress.v1',
    'browser_import.result.v1','browser_import.failure.v1'].includes(value.type)
  && value.request_id===binding.request_id && value.operation_nonce===binding.operation_nonce
  && value.connector_session_id===binding.connector_session_id && serialized(value)<=131072; }
function validConnectorPort(port,id) { try { const url=new URL(port.sender?.url); const expected=new URL(
    `chrome-extension://${id}/apps/browser-session-import/chrome-extension/connector.html`);
  return port.name===connectorName && port.sender?.id===id && Number.isSafeInteger(port.sender?.tab?.id)
    && port.sender.tab.incognito===false && url.origin===expected.origin && url.pathname===expected.pathname;
  } catch { return false; } }
function validWebPort(port) { try { const url=new URL(port.sender?.url); return port.name===externalName
    && browserImportWebOrigins.includes(url.origin) && port.sender?.origin===url.origin
    && port.sender?.id===undefined && Number.isSafeInteger(port.sender?.tab?.id)
    && port.sender.tab.incognito===false; } catch { return false; } }
function source(value) { return plain(value) && exact(value,['current_profile','hostname','store_id'])
  && value.current_profile===true && value.store_id==='0' && hostname(value.hostname); }
function extensionIdValue(value) { return typeof value==='string' && /^[a-p]{32}$/.test(value); }
function identifier(value) { return typeof value==='string' && /^[a-fA-F0-9]{32}$/.test(value); }
function relayKey(value) { try { const binary=atob(value); return binary.length===65 && binary.charCodeAt(0)===4; } catch { return false; } }
function hostname(value) { return typeof value==='string' && value.length>0 && value.length<=253
  && /^(?=.{1,253}$)(?:[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)*[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?$/.test(value); }
function serialized(value) { try { return JSON.stringify(value).length; } catch { return Infinity; } }
function plain(value) { return typeof value==='object' && value!==null && !Array.isArray(value); }
function exact(value,keys) { const actual=Object.keys(value); return actual.length===keys.length
  && keys.every(key=>Object.hasOwn(value,key)); }
function safePost(port,value) { try { port.postMessage(value); } catch { /* disconnected */ } }
function disconnect(port) { try { port.disconnect(); } catch { /* already disconnected */ } }
