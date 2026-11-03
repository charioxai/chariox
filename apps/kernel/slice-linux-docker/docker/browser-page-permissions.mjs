import { randomUUID } from 'node:crypto';
import { installPagePermissionShim } from './browser-page-permission-shim.mjs';

export const permissionBinding = '__charioxPagePermission';
// A registered symbol, so the page has no named global to find.
const responseSlot = 'chariox.pagePermissionResponse';
const allowed = new Set(['geolocation', 'notifications', 'clipboard-read']);
const blocked = new Set(['camera', 'microphone', 'display-capture']);
const invalid = () => Object.assign(new Error('Browser permission request is no longer pending; read the current browser state.'), {code:'browser_dialog_invalid'});

// Owned by the kernel browser's CDP client. Decisions live only for this browser
// session and an HTTP(S) origin; no profile permission preferences are written.
export class BrowserPagePermissions {
  constructor(owner, {timeoutMs = 30000} = {}) {
    this.owner = owner; this.timeoutMs = timeoutMs;
    this.contexts = new Map(); this.requests = new Map(); this.decisions = new Map();
  }
  async initialize(connection) {
    await connection.send('Browser.resetPermissions');
    for (const name of [...allowed, ...blocked]) await connection.send('Browser.setPermission', {permission:{name}, setting:'denied'});
  }
  async install(connection, sessionId) {
    await connection.send('Runtime.addBinding', {name:permissionBinding}, sessionId);
    await connection.send('Page.addScriptToEvaluateOnNewDocument', {
      source:`(${installPagePermissionShim})(${JSON.stringify(permissionBinding)},${JSON.stringify(responseSlot)})`, runImmediately:true,
    }, sessionId);
  }
  observe(message) {
    const {method, params = {}, sessionId} = message ?? {};
    const target = this.owner.targetsBySession.get(sessionId);
    if (method === 'Runtime.executionContextCreated' && target) {
      const c = params.context;
      if (c?.auxData?.isDefault && typeof c.origin === 'string' && /^https?:\/\//.test(c.origin)) {
        this.contexts.set(`${sessionId}:${c.id}`, {target, sessionId, contextId:c.id, origin:new URL(c.origin).origin, frameId:c.auxData.frameId});
      }
    } else if (method === 'Runtime.executionContextDestroyed') this.dropContext(`${sessionId}:${params.executionContextId}`);
    else if (method === 'Runtime.executionContextsCleared') {
      for (const [key, c] of this.contexts) if (c.sessionId === sessionId) this.dropContext(key);
    } else if (method === 'Page.frameNavigated') {
      for (const [key, c] of this.contexts) if (c.target === target && (!params.frame?.parentId || c.frameId === params.frame.id)) this.dropContext(key);
    } else if (['Target.targetDestroyed', 'Target.targetCrashed', 'Inspector.targetCrashed'].includes(method)) this.dropTarget(target ?? params.targetId);
    else if (method === 'Runtime.bindingCalled' && params.name === permissionBinding) {
      const context = this.contexts.get(`${sessionId}:${params.executionContextId}`);
      if (!context || typeof params.payload !== 'string' || params.payload.length > 256) return;
      let request; try { request = JSON.parse(params.payload); } catch { return; }
      const {id, permission, op} = request ?? {};
      if (typeof id !== 'string' || !/^[\w-]{1,64}$/.test(id) || !['request', 'query'].includes(op) || !allowed.has(permission) && !blocked.has(permission)) return;
      const documentId = this.owner.documentIdsByTarget.get(target);
      if (!documentId) return;
      const r = {...context, id, permission, documentId};
      const setting = blocked.has(permission) ? 'denied' : this.decisions.get(`${context.origin}:${permission}`);
      if (op === 'query' || setting) { void this.respond(r, setting ?? 'prompt'); return; }
      const existing = [...this.requests.values()].find(r => r.origin === context.origin && r.permission === permission);
      if (existing) {
        if (existing.waiters.length < 64) existing.waiters.push(r);
        else void this.respond(r, 'denied');
        return;
      }
      if (this.requests.size >= 64 || [...this.requests.values()].filter(r => r.target === target).length >= 16) { void this.respond(r, 'denied'); return; }
      const dialogId = randomUUID();
      const dialog = {dialog_id:dialogId, target_id:target, document_id:documentId, kind:'permission', message:permission, origin:context.origin};
      const timer = setTimeout(() => { void this.answer(dialog, false).catch(() => {}); }, this.timeoutMs); timer.unref?.();
      this.requests.set(dialogId, {...r, timer, waiters:[r]});
      this.owner.dialogDefaults.permissions.set(dialogId, dialog);
      this.changed();
    }
  }
  async answer(dialog, accept) {
    const r = this.requests.get(dialog?.dialog_id);
    if (!r || r.target !== dialog.target_id || r.documentId !== dialog.document_id || r.answering) throw invalid();
    r.answering = true;
    const setting = accept ? 'granted' : 'denied';
    try {
      // Apply the native grant before letting the page call the real API.
      await this.owner.connection.send('Browser.setPermission', {permission:{name:r.permission}, setting, origin:r.origin});
      if (this.requests.get(dialog.dialog_id) !== r) throw invalid();
      this.decisions.set(`${r.origin}:${r.permission}`, setting);
      await this.retire(dialog.dialog_id, setting);
      const replied = new Set(r.waiters.map(w => `${w.sessionId}:${w.contextId}`));
      await Promise.all([...this.contexts.values()].filter(c => c.origin === r.origin && !replied.has(`${c.sessionId}:${c.contextId}`)).map(c =>
        this.owner.connection?.send('Runtime.evaluate', {expression:`globalThis[Symbol.for(${JSON.stringify(responseSlot)})]?.(null,${JSON.stringify(setting)},${JSON.stringify(r.permission)})`, contextId:c.contextId, returnByValue:true}, c.sessionId).catch(() => {})));

    } catch (error) {
      await this.retire(dialog.dialog_id); throw error;
    }
  }
  async respond(r, setting) {
    // Never evaluate in a replacement document (context ids are CDP-owned).
    if (!this.contexts.has(`${r.sessionId}:${r.contextId}`)) return;
    await this.owner.connection?.send('Runtime.evaluate', {expression:`globalThis[Symbol.for(${JSON.stringify(responseSlot)})]?.(${JSON.stringify(r.id)},${JSON.stringify(setting)})`, contextId:r.contextId, returnByValue:true}, r.sessionId).catch(() => {});
  }
  retire(id, setting = 'denied') {
    const r = this.requests.get(id); if (!r) return Promise.resolve();
    clearTimeout(r.timer); this.requests.delete(id); this.owner.dialogDefaults.permissions.delete(id); this.changed();
    return Promise.all(r.waiters.map(waiter => this.respond(waiter, setting)));
  }
  dropContext(key) {
    const c = this.contexts.get(key); if (!c) return;
    this.contexts.delete(key);
    for (const [id, r] of this.requests) {
      if (r.sessionId === c.sessionId && r.contextId === c.contextId) void this.retire(id);
      else r.waiters = r.waiters.filter(w => w.sessionId !== c.sessionId || w.contextId !== c.contextId);
    }
  }
  dropTarget(target) {
    for (const [key, c] of this.contexts) if (c.target === target) this.dropContext(key);
    for (const [id, r] of this.requests) if (r.target === target) this.retire(id);
  }
  changed() { this.owner.onPermissionChanged?.(); }
  clear() { for (const id of this.requests.keys()) this.retire(id); this.contexts.clear(); this.decisions.clear(); }
}
