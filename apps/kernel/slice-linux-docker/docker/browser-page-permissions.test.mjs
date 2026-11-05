// MP-08/MP-09/MP-10/MP-11 BROWSER09: shared permission decisions deny by default.
import test from 'node:test';
import assert from 'node:assert/strict';
import { BrowserDialogDefaults } from './browser-controller-dialogs.mjs';
import { BrowserPagePermissions, permissionBinding } from './browser-page-permissions.mjs';

function fixture(timeoutMs = 30000) {
  const calls = [];
  const connection = { send: async (method, params, sessionId) => { calls.push({method, params, sessionId}); return {}; } };
  const owner = { connection, dialogDefaults: new BrowserDialogDefaults(), targetsBySession: new Map([['s', 't']]),
    documentIdsByTarget: new Map([['t', 'd']]), onPermissionChanged() {} };
  const permissions = new BrowserPagePermissions(owner, {timeoutMs});
  permissions.observe({method:'Runtime.executionContextCreated', sessionId:'s', params:{context:{id:7, origin:'https://example.test', auxData:{isDefault:true, frameId:'f'}}}});
  const request = (permission, id = 'request-1', op = 'request') => permissions.observe({method:'Runtime.bindingCalled', sessionId:'s', params:{name:permissionBinding, executionContextId:7, payload:JSON.stringify({id, permission, op})}});
  return { owner, permissions, calls, request };
}

test('MP-08/09/10/11 permission allow is explicit, origin scoped and shared with normal dialogs', async () => {
  const f = fixture();
  try {
    f.request('geolocation');
    const [dialog] = f.owner.dialogDefaults.pending();
    assert.equal(dialog.kind, 'permission'); assert.equal(dialog.message, 'geolocation');
    assert.equal(dialog.origin, 'https://example.test'); assert.equal(dialog.document_id, 'd');
    assert.ok(!f.calls.some(c => c.method === 'Browser.setPermission'));
    await f.permissions.answer(dialog, true);
    assert.deepEqual(f.owner.dialogDefaults.pending(), []);
    assert.deepEqual(f.calls.find(c => c.method === 'Browser.setPermission').params, {permission:{name:'geolocation'}, setting:'granted', origin:'https://example.test'});
    f.request('geolocation', 'request-2'); await new Promise(r => setImmediate(r));
    assert.deepEqual(f.owner.dialogDefaults.pending(), []);
    assert.equal(f.calls.filter(c => c.method === 'Browser.setPermission').length, 1);
    await assert.rejects(f.permissions.answer(dialog, true), {code:'browser_dialog_invalid'});
  } finally { f.permissions.clear(); }
});

test('MP-08/09/10/11 timeout, explicit deny and unavailable devices release page requests', async () => {
  const f = fixture(15);
  try {
    f.request('notifications'); await new Promise(r => setTimeout(r, 40));
    assert.deepEqual(f.owner.dialogDefaults.pending(), []);
    assert.equal(f.calls.find(c => c.method === 'Runtime.evaluate').params.expression.includes('denied'), true);
    assert.ok(!f.calls.some(c => c.params?.setting === 'granted'));
    f.request('clipboard-read', 'clip');
    await f.permissions.answer(f.owner.dialogDefaults.pending()[0], false);
    for (const name of ['camera', 'microphone', 'display-capture']) f.request(name, name);
    await new Promise(r => setImmediate(r));
    assert.deepEqual(f.owner.dialogDefaults.pending(), []);
    assert.ok(!f.calls.some(c => c.params?.setting === 'granted'));
  } finally { f.permissions.clear(); }
});

test('MP-08/09/10/11 navigation and target close retire requests; query never opens a dialog', async () => {
  const f = fixture();
  try {
    f.request('notifications', 'q', 'query'); await new Promise(r => setImmediate(r));
    assert.deepEqual(f.owner.dialogDefaults.pending(), []);
    f.request('geolocation');
    const [old] = f.owner.dialogDefaults.pending();
    f.permissions.observe({method:'Page.frameNavigated', sessionId:'s', params:{frame:{id:'f', loaderId:'new'}}});
    assert.deepEqual(f.owner.dialogDefaults.pending(), []);
    await assert.rejects(f.permissions.answer(old, true), {code:'browser_dialog_invalid'});
    f.request('notifications', 'n');
    f.permissions.dropTarget('t'); assert.deepEqual(f.owner.dialogDefaults.pending(), []);
  } finally { f.permissions.clear(); }
});

test('MP-08/09/10/11 binding requests use the CDP execution context origin and reject unknown worlds', () => {
  const f = fixture();
  try {
    f.permissions.observe({method:'Runtime.bindingCalled',sessionId:'s',params:{name:permissionBinding,executionContextId:8,payload:JSON.stringify({op:'request',id:'forged',permission:'geolocation',origin:'https://forged.test'})}});
    assert.deepEqual(f.owner.dialogDefaults.pending(), []);
  } finally { f.permissions.clear(); }
});


test('MP-08/09/10/11 concurrent calls share one dialog and one origin decision', async () => {
  const f = fixture();
  try {
    f.request('geolocation', 'first'); f.request('geolocation', 'second');
    assert.equal(f.owner.dialogDefaults.pending().length, 1);
    await f.permissions.answer(f.owner.dialogDefaults.pending()[0], true);
    assert.equal(f.calls.filter(c => c.method === 'Browser.setPermission').length, 1);
    const replies = f.calls.filter(c => c.method === 'Runtime.evaluate');
    assert.equal(replies.length, 2);
    assert.ok(replies.every(c => c.params.expression.includes('granted')));
  } finally { f.permissions.clear(); }
});


test('MP-08/09/10/11 failed native grant denies the page and leaves no cached allow', async () => {
  const f = fixture();
  try {
    f.owner.connection.send = async method => { if (method === 'Browser.setPermission') throw new Error('CDP unavailable'); return {}; };
    f.request('geolocation');
    await assert.rejects(f.permissions.answer(f.owner.dialogDefaults.pending()[0], true), /CDP unavailable/);
    assert.deepEqual(f.owner.dialogDefaults.pending(), []);
    assert.equal(f.permissions.decisions.size, 0);
  } finally { f.permissions.clear(); }
});


test('MP-08/09/10/11 an origin decision updates permission status in its other live pages', async () => {
  const f = fixture();
  try {
    f.permissions.observe({method:'Runtime.executionContextCreated',sessionId:'s',params:{context:{id:9,origin:'https://example.test',auxData:{isDefault:true,frameId:'other'}}}});
    f.request('notifications'); await f.permissions.answer(f.owner.dialogDefaults.pending()[0], true);
    assert.ok(f.calls.some(c => c.method === 'Runtime.evaluate' && c.params.contextId === 9 && c.params.expression.includes('(null,"granted","notifications")')));
  } finally { f.permissions.clear(); }
});
