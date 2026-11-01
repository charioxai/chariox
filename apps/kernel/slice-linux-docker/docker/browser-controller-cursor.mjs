// MP-08/MP-10: the cursor the page shows at a point (protocol 920), for viewers that show the page as video (whole-page
// video, navigation preview). Read in the kernel's isolated world of the live document, never the page's own world.
import { BrowserSnapshotError } from './browser-controller-snapshot.mjs';

// The CSS cursor keywords a viewer can show without loading anything; a url() cursor shows its fallback keyword.
export const POINTER_CURSORS = new Set(['default', 'none', 'context-menu', 'help', 'pointer', 'progress', 'wait', 'cell', 'crosshair', 'text', 'vertical-text', 'alias', 'copy', 'move', 'no-drop', 'not-allowed', 'grab', 'grabbing', 'e-resize', 'n-resize', 'ne-resize', 'nw-resize', 's-resize', 'se-resize', 'sw-resize', 'w-resize', 'ew-resize', 'ns-resize', 'nesw-resize', 'nwse-resize', 'col-resize', 'row-resize', 'all-scroll', 'zoom-in', 'zoom-out']);

// Runs in a document's renderer: the hit element through open shadow roots and same-origin frames, and Chrome's `auto`
// (an I-beam over selectable text and editable fields, else the arrow). A cross-origin frame's content is another
// document: the answer is then `{ element, x, y }`, the frame element and the point in its own coordinates.
function cursorAt(x, y) {
  let doc = document, element = doc.elementFromPoint(x, y);
  for (let depth = 0; element && depth < 32; depth++) {
    const inner = element.shadowRoot?.elementFromPoint(x, y);
    if (inner && inner !== element) { element = inner; continue; }
    let nested = null;
    const frame = /^i?frame$/.test(element.localName);
    try { nested = frame ? element.contentDocument : null; } catch { nested = null; }
    if (!frame) break;
    // The frame's content box maps to its document at the frame's CSS scale (a scaled frame shows its document scaled).
    const box = element.getBoundingClientRect(), style = getComputedStyle(element);
    const sx = element.offsetWidth > 0 ? box.width / element.offsetWidth : 1, sy = element.offsetHeight > 0 ? box.height / element.offsetHeight : 1;
    x = (x - box.left - (element.clientLeft + (parseFloat(style.paddingLeft) || 0)) * sx) / sx; y = (y - box.top - (element.clientTop + (parseFloat(style.paddingTop) || 0)) * sy) / sy;
    if (!nested) return { element, x, y };
    doc = nested; element = doc.elementFromPoint(x, y);
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

const MAX_FRAME_DEPTH = 4;

// A frame's own document: its own session when it is out of process, else the page's, in an isolated world of its own.
async function frameWorld(browser, connection, sessionId, element, point) {
  try {
    const { node } = await connection.send('DOM.describeNode', { objectId: element.objectId }, sessionId);
    if (!node?.frameId || !Number.isFinite(point.x) || !Number.isFinite(point.y)) return 'default';
    const frameSession = browser.frameSession?.(node.frameId, connection) ?? sessionId;
    const { executionContextId } = await connection.send('Page.createIsolatedWorld', { frameId: node.frameId, worldName: 'chariox-cursor', grantUniveralAccess: false }, frameSession);
    return { sessionId: frameSession, contextId: executionContextId, ...point };
  } catch { return 'default'; } finally { await connection.send('Runtime.releaseObject', { objectId: element.objectId }, sessionId).catch(() => {}); }
}

async function cursorIn(browser, connection, sessionId, contextId, x, y) {
  for (let depth = 0; depth < MAX_FRAME_DEPTH; depth++) {
    const reply = await connection.send('Runtime.evaluate', { expression: `(${cursorAt})(${x},${y})`, contextId }, sessionId);
    const answer = reply?.result;
    if (answer?.type === 'string') return answer.value;
    if (!answer?.objectId) return 'default';
    const { result = [] } = await connection.send('Runtime.getProperties', { objectId: answer.objectId, ownProperties: true }, sessionId);
    await connection.send('Runtime.releaseObject', { objectId: answer.objectId }, sessionId).catch(() => {});
    const property = name => result.find(item => item.name === name)?.value;
    const next = await frameWorld(browser, connection, sessionId, property('element') ?? {}, { x: property('x')?.value, y: property('y')?.value });
    if (typeof next === 'string') return next;
    ({ sessionId, contextId, x, y } = next);
  }
  return 'default';
}

export async function readPointerCursor(browser, { target_id, document_id, x, y }) {
  if (!Number.isFinite(x) || !Number.isFinite(y)) throw new Error('MP-08: pointer cursor needs a point');
  const connection = await browser.ensureConnection(), sessionId = await browser.ensureTargetSession(connection, target_id);
  const frame = (await connection.send('Page.getFrameTree', {}, sessionId)).frameTree?.frame;
  if (!frame || frame.loaderId !== document_id) throw new BrowserSnapshotError('stale_document_reference', 'browser document changed before the cursor read');
  const world = await browser.ensureFocusWorld(connection, sessionId, target_id, frame);
  let cursor;
  try { cursor = await cursorIn(browser, connection, sessionId, world.contextId, x, y); }
  catch (error) {
    browser.focusWorldsByTarget.delete(target_id);
    if (/Cannot find context with specified id|Execution context was destroyed/.test(error?.message ?? '')) throw new BrowserSnapshotError('stale_document_reference', 'browser document changed during the cursor read');
    throw error;
  }
  return POINTER_CURSORS.has(cursor) ? cursor : 'default';
}
