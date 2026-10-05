// MP-11: retained input uses a native allow-list and value-free activation classification.
import { randomUUID } from "node:crypto";
import { assertCurrentDocument } from "./browser-controller-actions.mjs";
import { protectedHostRegions } from "./kernel-browser-region-protection.mjs";

// DOM classification functions execute in a CDP isolated world, using native DOM prototypes.
// A label is only routine when it names one of these bounded navigation/search
// actions. Unknown, icon-only and mixed labels require live focus.
function routineAction(node) {
  if (node?.nodeType !== 1) return false;
  const labels = [node.getAttribute('aria-label'), node.textContent,
    ['button', 'submit', 'image', 'reset'].includes(node.type) ? node.value : '', node.getAttribute('alt')]
    .filter(label => label?.trim()).map(label => label.trim().replace(/\s+/g, ' '));
  return labels.length > 0 && labels.every(label =>
    /^(search|find|next|previous|back|forward|expand|collapse|show more|show less)$/i.test(label));
}

function inputTargets(input, routine) {
  let e = input.kind === 'click' ? document.elementFromPoint(input.x, input.y) : document.activeElement;
  if (!e) return null;
  while (e.shadowRoot?.activeElement) e = e.shadowRoot.activeElement;
  const protectedSelector = '[data-chariox-sensitive],[data-chariox-observation-protected],input[type="password"],[autocomplete="one-time-code"],[autocomplete^="cc-"],iframe,frame';
  const form = e.form || e.closest('form');
  if (e.matches(protectedSelector) || e.closest(protectedSelector) || e.shadowRoot || form?.querySelector(protectedSelector)) return null;
  const editable = !e.disabled && !e.readOnly && (e.tagName === 'TEXTAREA' || e.isContentEditable ||
    (e.tagName === 'INPUT' && ['text', 'search', 'email', 'url', 'tel', 'number'].includes(e.type)));
  // MP-11: retained authority permits native editing, not arbitrary keyboard
  // shortcuts. Do not try to infer the effects of page/delegated scripts.
  if (input.kind === 'text') return editable ? [] : null;
  if (input.kind === 'key' && editable &&
    (['Space', 'Backspace', 'Delete', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(input.key) ||
      (typeof input.key === 'string' && /^[^\p{C}]$/u.test(input.key)))) return [];
  const activation = input.kind === 'click' || (input.kind === 'key' && ['Enter', 'Space'].includes(input.key));
  if (!activation) return null;
  const targets = new Set();
  const addPath = node => {
    for (; node; node = node.parentNode) {
      targets.add(node);
      if (targets.size > 256) return false;
    }
    targets.add(window);
    return true;
  };
  if (!addPath(e)) return null;
  if (input.key === 'Enter' && e.tagName === 'INPUT' && !['submit','image','button','reset','checkbox','radio'].includes(e.type)) {
    if (!form) return null;
    const submit = [...e.getRootNode().querySelectorAll('button,input')].find(control =>
      control.form === form && ['submit','image'].includes(control.type));
    // Include the real native default action, even outside the form element.
    if (!submit || !addPath(submit)) return null;
  }
  if (form && !addPath(form)) return null;
  const actionable = 'button,input[type="button"],input[type="submit"],input[type="image"],input[type="reset"],input[type="checkbox"],input[type="radio"],select,a[href],[role="button"],[role="link"],[role="menuitem"],[role="tab"],[role="switch"],[role="checkbox"],[role="option"],label,summary';
  for (const target of targets) {
    if (target.nodeType !== 1) continue;
    if (target.matches(protectedSelector) || target.shadowRoot) return null;
    if (target.matches(actionable) && !routine(target)) return null;
    if (target.tagName === 'LABEL') {
      if (!target.control || !routine(target.control) || !addPath(target.control)) return null;
    }
  }
  return [...targets];
}

export async function sensitiveHostInput(browser, tab, input) {
  if (input.kind === 'scroll' || (input.kind === 'key' && ['Tab', 'Shift+Tab'].includes(input.key))) return false;
  const { connection, sessionId } = await browser.resolvePageTarget(tab.target_id);
  await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
  const group = `chariox-input-protection-${randomUUID()}`;
  try {
    const { frameTree } = await connection.send('Page.getFrameTree', {}, sessionId);
    const { executionContextId } = await connection.send('Page.createIsolatedWorld', {
      frameId: frameTree.frame.id, worldName: 'chariox-host-protection', grantUniveralAccess: false,
    }, sessionId);
    const targetInput = { kind: input.kind, key: input.key, x: input.x, y: input.y };
    const scan = await connection.send('Runtime.evaluate', {
      contextId: executionContextId, objectGroup: group,
      expression: `(${inputTargets})(${JSON.stringify(targetInput)}, ${routineAction})`, returnByValue: false,
    }, sessionId);
    if (scan.exceptionDetails || !scan.result?.objectId || scan.result.subtype !== 'array') return true;
    const { result: targets } = await connection.send('Runtime.getProperties', {
      objectId: scan.result.objectId, ownProperties: true,
    }, sessionId);
    if (!Array.isArray(targets)) return true;
    for (const target of targets.filter(property => /^\d+$/.test(property.name))) {
      const objectId = target.value?.objectId;
      if (!objectId) return true;
      // DOMDebugger only reports listeners in the object's world. Resolve native
      // nodes into the main world; do not evaluate page-controlled prototypes.
      let listenerObject;
      let backendNodeId;
      if (target.value.subtype === 'node') {
        const { node } = await connection.send('DOM.describeNode', { objectId }, sessionId);
        backendNodeId = node?.backendNodeId;
        if (!backendNodeId) return true;
        listenerObject = (await connection.send('DOM.resolveNode', { backendNodeId, objectGroup: group }, sessionId)).object;
      } else if (target.value.className === 'Window') {
        listenerObject = (await connection.send('Runtime.evaluate', {
          expression: 'this', objectGroup: group, returnByValue: false,
        }, sessionId)).result;
      }
      if (!listenerObject?.objectId) return true;
      // Include delegated listeners on every ancestor through document/window.
      const { listeners } = await connection.send('DOMDebugger.getEventListeners', {
        objectId: listenerObject.objectId, depth: 1,
      }, sessionId);
      if (!Array.isArray(listeners)) return true;
      if (listeners.some(listener => typeof listener?.type !== 'string')) return true;
      if (listeners.some(listener => (backendNodeId === undefined || listener.backendNodeId === undefined || listener.backendNodeId === backendNodeId)
        && /^(click|dblclick|auxclick|contextmenu|mouse.*|pointer.*|touch.*|key.*|submit)$/.test(listener.type))) {
        const classification = await connection.send('Runtime.callFunctionOn', {
          objectId, functionDeclaration: `function() { return (${routineAction})(this); }`, returnByValue: true,
        }, sessionId);
        if (classification.exceptionDetails || classification.result?.value !== true) return true;
      }
    }
    const regions = await protectedHostRegions(connection, sessionId);
    return input.kind === 'click'
      ? regions.some(r => input.x >= r.x && input.x <= r.x + r.width && input.y >= r.y && input.y <= r.y + r.height)
      : regions.length > 0;
  } catch { return true; }
  finally { await connection.send('Runtime.releaseObjectGroup', { objectGroup: group }, sessionId).catch(() => {}); }
}
