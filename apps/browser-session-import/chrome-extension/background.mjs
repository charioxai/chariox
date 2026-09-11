import {PermissionGrantCoordinator,createChromeSessionPermissionStateStore}
  from './permission-coordinator.mjs';

const cleanupAlarm = 'browser-import-permission-cleanup-v1';
const permissionCoordinator = new PermissionGrantCoordinator(
  details => chrome.permissions.remove(details),
  {stateStore:createChromeSessionPermissionStateStore(chrome)});
const ownerAlive = async tabId => {
  try { return Number.isSafeInteger((await chrome.tabs.get(tabId))?.id); }
  catch { return false; }
};
const recover = () => permissionCoordinator.recover({ownerAlive});
const settle = operation => { void Promise.resolve(operation).catch(() => {}); };

chrome.permissions.onAdded.addListener(details => settle(permissionCoordinator.observeAdded(details)));
chrome.tabs.onRemoved.addListener(tabId => settle(permissionCoordinator.releaseOwner(tabId)));
chrome.alarms.onAlarm.addListener(alarm => {
  if (alarm?.name === cleanupAlarm) settle(recover());
});
settle(chrome.alarms.create(cleanupAlarm,{periodInMinutes:0.5}));
settle(recover());

chrome.action.onClicked.addListener(tab => {
  if (!Number.isSafeInteger(tab?.id) || tab.incognito !== false) return;
  const page = new URL(chrome.runtime.getURL('apps/browser-session-import/chrome-extension/connector.html'));
  page.searchParams.set('sourceTabId',String(tab.id));
  void chrome.tabs.create({url:page.href});
});

chrome.runtime.onConnect.addListener(port => {
  if (port.name !== 'browser-import-permission-lease') return;
  const ownerTabId = port.sender?.tab?.id;
  let sender;
  try { sender = new URL(port.sender?.url); } catch { /* rejected below */ }
  const connector = new URL(chrome.runtime.getURL('apps/browser-session-import/chrome-extension/connector.html'));
  if (!Number.isSafeInteger(ownerTabId) || ownerTabId < 0 || sender?.origin !== connector.origin
      || sender.pathname !== connector.pathname) {
    try { port.disconnect(); } catch { /* already disconnected */ }
    return;
  }
  port.onMessage.addListener(message => void (async () => {
    try {
      const operationId = message?.operation_id;
      if (message?.kind === 'reserve') {
        await permissionCoordinator.reserve(operationId,message.permission,message.preexisting,{ownerTabId});
      } else if (message?.kind === 'activate') {
        await permissionCoordinator.activate(operationId,ownerTabId);
      } else if (message?.kind === 'release') {
        await permissionCoordinator.release(operationId,ownerTabId);
      } else throw new Error();
      port.postMessage({id:message?.id,ok:true});
    } catch { try { port.postMessage({id:message?.id,ok:false}); } catch { /* disconnected */ } }
  })());
  // A worker suspension also disconnects this port, so disconnect is not authority
  // that the owning connector tab ended. Explicit release, tab removal, and recovery are.
});
