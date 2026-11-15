// Saved-card checkout commits use the owner's fill approval once. Kept in the
// kernel's browser state so a lost reply or controller restart cannot pay twice.
import { AgentToolRefusal } from './kernel-browser-refusal.mjs';
import { open, readFile } from 'node:fs/promises';
import path from 'node:path';
import { BrowserActionError } from './browser-controller-actions.mjs';
import { topLevelNodeId, withLiveNode } from './kernel-browser-node-ref.mjs';

export class PaymentActions {
  constructor(root) { this.root = root; this.tabs = new Map(); this.sent = new Set(); this.loaded = null; }
  async load() {
    this.loaded ??= readFile(path.join(this.root, 'payment-submissions.jsonl'), 'utf8').then(text => {
      for (const line of text.split('\n').filter(Boolean)) this.sent.add(JSON.parse(line));
    }, error => { if (error.code !== 'ENOENT') throw error; });
    await this.loaded;
  }
  async reserve(id) {
    // Two tabs can race on one approval; reserve in memory before the first await.
    if (this.sent.has(id)) throw new AgentToolRefusal('payment_already_sent');
    this.sent.add(id);
    const file = await open(path.join(this.root, 'payment-submissions.jsonl'), 'a', 0o600);
    try { await file.writeFile(JSON.stringify(id) + '\n'); await file.sync(); }
    finally { await file.close(); }
    const directory = await open(this.root, 'r');
    try { await directory.sync(); } finally { await directory.close(); }
  }
  register(tab, approval) {
    if (!approval || typeof approval.id !== 'string' || typeof approval.origin !== 'string' ||
        !Number.isSafeInteger(approval.expires_at_ms) || new URL(tab.url).origin !== approval.origin) {
      throw new AgentToolRefusal('payment_approval_required');
    }
    this.tabs.set(tab.tab_id, { ...approval });
  }
  async beforeInput(browser, tab, command) {
    const saved = this.tabs.get(tab.tab_id), input = command.input;
    if (!saved || !(command._agent_input || String(command.observed_by).startsWith('agent:')) ||
        !(input?.kind === 'click' || input?.kind === 'key' && ['Enter', 'Space', ' '].includes(input.key))) return;
    const target = await browser.paymentInputTarget(tab, input);
    if (target.document_id !== tab.document_id) throw new BrowserActionError('stale_document_reference', 'The checkout changed. Observe it again.');
    if (!target.payment) return;
    await this.load();
    if (this.sent.has(saved.id)) throw new AgentToolRefusal('payment_already_sent');
    if (saved.expires_at_ms <= Date.now() || saved.origin !== target.origin ||
        !command._vault_payment_approvals?.some(approval => approval.id === saved.id && approval.origin === target.origin)) {
      throw new AgentToolRefusal('payment_approval_required');
    }
    // Reserve before dispatch, including when its acknowledgement is lost.
    await this.reserve(saved.id);
  }
}

// Observe only the action's label and form semantics, never input values. Pointer
// and keyboard input resolve the actual element; snapshot refs use its backend id.
export async function paymentInputTarget(browser, tab, input) {
  const { connection, sessionId } = await browser.resolvePageTarget(tab.target_id);
  let objectId;
  if (input.target) {
    const resolved = await withLiveNode(() => connection.send('DOM.resolveNode', { backendNodeId: topLevelNodeId(input.target) }, sessionId));
    objectId = resolved.object?.objectId;
  } else {
    const expression = input.kind === 'click'
      ? `document.elementFromPoint(${Number(input.x)},${Number(input.y)})`
      : 'document.activeElement';
    const resolved = await connection.send('Runtime.evaluate', { expression }, sessionId);
    objectId = resolved.result?.objectId;
  }
  if (!objectId) throw new BrowserActionError('stale_element_reference', 'The checkout action is unavailable. Observe it again.');
  try {
    const result = await connection.send('Runtime.callFunctionOn', { objectId, functionDeclaration: `function(key) {
      const button = this.closest('button,input[type=submit],input[type=button],[role=button],a') || this;
      const label = element => (element.getAttribute('aria-label') || element.innerText || element.getAttribute('value') || '').trim();
      const pay = element => /^(pay(?:ment)?(?:\\s|$)|place\\s+(?:your\\s+)?order|buy(?:\\s|$)|purchase(?:\\s|$)|subscribe(?:\\s|$)|donate(?:\\s|$)|book(?:\\s|$)|(?:confirm|complete|submit)\\s+(?:payment|purchase|order))/i.test(label(element));
      const form = this.form || this.closest('form');
      const payment = pay(button) || key === 'Enter' && !!form && [...form.querySelectorAll('button,input[type=submit],[role=button]')].some(pay);
      return { payment, origin: this.ownerDocument.location.origin };
    }`, arguments: [{ value: input.kind === 'key' ? input.key : null }], returnByValue: true }, sessionId);
    if (result.exceptionDetails || !result.result?.value || typeof result.result.value.payment !== 'boolean') throw new AgentToolRefusal('payment_target_unavailable');
    return { ...result.result.value, document_id: tab.document_id };
  } finally { await connection.send('Runtime.releaseObject', { objectId }, sessionId).catch(() => {}); }
}
