// MP-08/MP-10/MP-11: an agent's screenshot of one element, a viewport region or the whole page.
// An element or region is cut from the same masked viewport capture a plain screenshot returns, so Vault masking is
// unchanged. A whole page has no mask geometry, so it is refused while any protected target is on the page.
import { assertCurrentDocument } from './browser-controller-actions.mjs';
import { AgentToolRefusal, UserDomainRefusal } from './kernel-browser-refusal.mjs';
import { topLevelNodeId, withLiveNode } from './kernel-browser-node-ref.mjs';

export const REGION_KEYS = ['node_ref', 'clip', 'full_page'];
const MAX_SIDE = 16000;
const IMAGE_LIMIT = 4 * 1024 * 1024;

function checkedClip(clip, page) {
  const fields = ['x', 'y', 'width', 'height'];
  if (!clip || typeof clip !== 'object' || !fields.every(key => Number.isInteger(clip[key])) ||
      Object.keys(clip).some(key => !fields.includes(key)) ||
      clip.x < 0 || clip.y < 0 || clip.width < 1 || clip.height < 1 ||
      clip.x + clip.width > page.width || clip.y + clip.height > page.height) throw new AgentToolRefusal('clip_invalid');
  return { x: clip.x, y: clip.y, width: clip.width, height: clip.height };
}

async function elementClip(host, tab, nodeRef, page) {
  const backendNodeId = topLevelNodeId(nodeRef);
  const { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id);
  await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
  const { model } = await withLiveNode(async () => {
    await connection.send('DOM.scrollIntoViewIfNeeded', { backendNodeId }, sessionId);
    return connection.send('DOM.getBoxModel', { backendNodeId }, sessionId);
  });
  const quad = model?.border;
  if (!Array.isArray(quad) || quad.length !== 8 || !quad.every(Number.isFinite)) throw new AgentToolRefusal('node_unavailable');
  const xs = [quad[0], quad[2], quad[4], quad[6]], ys = [quad[1], quad[3], quad[5], quad[7]];
  const left = Math.max(0, Math.floor(Math.min(...xs))), top = Math.max(0, Math.floor(Math.min(...ys)));
  const right = Math.min(page.width, Math.ceil(Math.max(...xs))), bottom = Math.min(page.height, Math.ceil(Math.max(...ys)));
  if (right <= left || bottom <= top) throw new AgentToolRefusal('node_not_visible');
  return { x: left, y: top, width: right - left, height: bottom - top };
}

async function fullPage(host, tab) {
  if (host.protection.targets.length || host.browser.fillTargets?.size) throw new UserDomainRefusal('not_granted');
  const { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id);
  await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
  const metrics = await connection.send('Page.getLayoutMetrics', {}, sessionId);
  const size = metrics.cssContentSize ?? metrics.contentSize;
  const width = Math.min(MAX_SIDE, Math.max(1, Math.ceil(size?.width)));
  const pageHeight = Math.max(1, Math.ceil(size?.height));
  const height = Math.min(MAX_SIDE, pageHeight);
  if (!Number.isFinite(width) || !Number.isFinite(pageHeight)) throw new AgentToolRefusal('image_too_large');
  // One image pixel per CSS pixel, whatever the page density.
  const scale = 1 / (host.scales.get(tab.tab_id) ?? host.viewport().device_scale_factor);
  const shoot = format => connection.send('Page.captureScreenshot', {
    format, ...(format === 'jpeg' ? { quality: 85 } : {}), captureBeyondViewport: true, optimizeForSpeed: true,
    clip: { x: 0, y: 0, width, height, scale },
  }, sessionId);
  const { format, data } = await host.sampleLane(tab).run('capture', () => host.browser.inputCapture.run(connection, sessionId, async () => {
    let shot = await shoot('png');
    if (typeof shot.data === 'string' && shot.data.length > IMAGE_LIMIT) return { format: 'jpeg', data: (await shoot('jpeg')).data };
    return { format: 'png', data: shot.data };
  }, { resumeFocus: false }));
  await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
  if (typeof data !== 'string' || data.length > IMAGE_LIMIT) throw new AgentToolRefusal('image_too_large');
  return { generation: host.generation, tab_id: tab.tab_id, document_id: tab.document_id, mime_type: `image/${format}`, data_base64: data,
    width, height, page_height: pageHeight, ...(pageHeight > height ? { truncated: true } : {}) };
}

export async function regionScreenshot(host, tab, command) {
  const chosen = REGION_KEYS.filter(key => command[key] !== undefined);
  if (chosen.length !== 1) throw new AgentToolRefusal('region_conflict');
  if (chosen[0] === 'full_page') {
    if (command.full_page !== true) throw new AgentToolRefusal('region_conflict');
    return fullPage(host, tab);
  }
  const page = host.pageSize(tab.tab_id);
  const clip = chosen[0] === 'clip' ? checkedClip(command.clip, page) : await elementClip(host, tab, command.node_ref, page);
  return { ...await host.displayScreenshot(tab, clip), clip };
}
