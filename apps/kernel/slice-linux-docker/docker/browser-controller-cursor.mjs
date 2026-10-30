// MP-08/MP-10: the cursor the page shows at a point (protocol 920), for viewers that show the page as video (whole-page
// video, navigation preview). Read in the kernel's isolated world of the live document, never the page's own world.
import { BrowserSnapshotError } from './browser-controller-snapshot.mjs';

// The CSS cursor keywords a viewer can show without loading anything; a url() cursor shows its fallback keyword.
export const POINTER_CURSORS = new Set(['default', 'none', 'context-menu', 'help', 'pointer', 'progress', 'wait', 'cell', 'crosshair', 'text', 'vertical-text', 'alias', 'copy', 'move', 'no-drop', 'not-allowed', 'grab', 'grabbing', 'e-resize', 'n-resize', 'ne-resize', 'nw-resize', 's-resize', 'se-resize', 'sw-resize', 'w-resize', 'ew-resize', 'ns-resize', 'nesw-resize', 'nwse-resize', 'col-resize', 'row-resize', 'all-scroll', 'zoom-in', 'zoom-out']);

// Runs in the page's renderer: the hit element through open shadow roots and same-origin frames, and Chrome's `auto`
// (an I-beam over selectable text and editable fields, else the arrow).
function cursorAt(x, y) {
  let doc = document, element = doc.elementFromPoint(x, y);
  for (let depth = 0; element && depth < 32; depth++) {
    const inner = element.shadowRoot?.elementFromPoint(x, y);
    if (inner && inner !== element) { element = inner; continue; }
    let nested = null;
    try { nested = /^i?frame$/.test(element.localName) ? element.contentDocument : null; } catch { nested = null; }
    if (!nested) break;
    const box = element.getBoundingClientRect();
    x -= box.left + element.clientLeft; y -= box.top + element.clientTop; doc = nested; element = doc.elementFromPoint(x, y);
  }
  if (!element) return 'default';
  const view = element.ownerDocument.defaultView, style = view.getComputedStyle(element);
  const cursor = style.cursor.split(',').at(-1).trim();
  if (cursor !== 'auto') return cursor;
  const textInput = element.localName === 'textarea' || element.localName === 'input' && !/^(button|checkbox|color|file|hidden|image|radio|range|reset|submit)$/.test(element.type);
  if (textInput || element.isContentEditable) return 'text';
  if (style.userSelect === 'none') return 'default';
  for (const node of element.childNodes) {
    if (node.nodeType !== 3 || !node.textContent.trim()) continue;
    const range = element.ownerDocument.createRange(); range.selectNodeContents(node);
    for (const rect of range.getClientRects()) if (x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom) return 'text';
  }
  return 'default';
}

export async function readPointerCursor(browser, { target_id, document_id, x, y }) {
  if (!Number.isFinite(x) || !Number.isFinite(y)) throw new Error('MP-08: pointer cursor needs a point');
  const connection = await browser.ensureConnection(), sessionId = await browser.ensureTargetSession(connection, target_id);
  const frame = (await connection.send('Page.getFrameTree', {}, sessionId)).frameTree?.frame;
  if (!frame || frame.loaderId !== document_id) throw new BrowserSnapshotError('stale_document_reference', 'browser document changed before the cursor read');
  const world = await browser.ensureFocusWorld(connection, sessionId, target_id, frame);
  let reply;
  try { reply = await connection.send('Runtime.evaluate', { expression: `(${cursorAt})(${x},${y})`, contextId: world.contextId, returnByValue: true }, sessionId); }
  catch (error) {
    browser.focusWorldsByTarget.delete(target_id);
    if (/Cannot find context with specified id|Execution context was destroyed/.test(error?.message ?? '')) throw new BrowserSnapshotError('stale_document_reference', 'browser document changed during the cursor read');
    throw error;
  }
  const cursor = reply?.result?.value;
  return POINTER_CURSORS.has(cursor) ? cursor : 'default';
}
