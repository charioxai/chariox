import {PermissionGrantCoordinator} from './permission-coordinator.mjs';

const permissionCoordinator = new PermissionGrantCoordinator(details => chrome.permissions.remove(details));
chrome.permissions.onAdded.addListener(details => permissionCoordinator.observeAdded(details));

chrome.action.onClicked.addListener(tab => {
  if (!Number.isSafeInteger(tab?.id) || tab.incognito !== false) return;
  const page = new URL(chrome.runtime.getURL('apps/browser-session-import/chrome-extension/connector.html'));
  page.searchParams.set('sourceTabId',String(tab.id));
  void chrome.tabs.create({url:page.href});
});

chrome.runtime.onConnect.addListener(port => {
  if (port.name !== 'browser-import-permission-lease') return;
  const operationId = crypto.randomUUID();
  let reserved = false;
  let released = false;
  const release = async () => {
    if (released) return;
    released = true;
    if (reserved) await permissionCoordinator.release(operationId);
  };
  port.onMessage.addListener(message => void (async () => {
    try {
      if (message?.kind === 'reserve' && !reserved) {
        permissionCoordinator.reserve(operationId,message.permission,message.preexisting);
        reserved = true;
      } else if (message?.kind === 'activate' && reserved) permissionCoordinator.activate(operationId);
      else if (message?.kind === 'release' && reserved) await release();
      else throw new Error();
      port.postMessage({id:message?.id,ok:true});
    } catch { try { port.postMessage({id:message?.id,ok:false}); } catch { /* disconnected */ } }
  })());
  port.onDisconnect.addListener(() => void release());
});
