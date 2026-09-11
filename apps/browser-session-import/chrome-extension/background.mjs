chrome.action.onClicked.addListener(tab => {
  if (!Number.isSafeInteger(tab?.id) || tab.incognito !== false) return;
  const page = new URL(chrome.runtime.getURL('apps/browser-session-import/chrome-extension/connector.html'));
  page.searchParams.set('sourceTabId',String(tab.id));
  void chrome.tabs.create({url:page.href});
});
