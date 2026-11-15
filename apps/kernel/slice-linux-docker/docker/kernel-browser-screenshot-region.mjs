// MP-08/MP-10/MP-11: an agent's screenshot of one element, a viewport region or the whole page.
// An element or region is cut from the same masked viewport capture a plain screenshot returns, so Vault masking is
// unchanged. A page that fits uses this same masked capture; taller pages cannot reuse viewport-clipped masks.
import { assertCurrentDocument } from './browser-controller-actions.mjs';
import { measurePageProtection } from './browser-protection-regions.mjs';
import { AgentToolRefusal } from './kernel-browser-refusal.mjs';
import { topLevelNodeId, withLiveNode } from './kernel-browser-node-ref.mjs';

export const REGION_KEYS = ['node_ref', 'clip', 'full_page'];
const MAX_SIDE = 16000;
const IMAGE_LIMIT = 4 * 1024 * 1024;
const FULL_PAGE_ATTEMPTS = [{ format: 'png', factor: 1 }, { format: 'jpeg', quality: 85, factor: 1 }, { format: 'jpeg', quality: 60, factor: 0.6 }, { format: 'jpeg', quality: 50, factor: 0.4 }];

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
  // A screenshot only looks: moving the page is the agent's admitted scroll (input scroll with this node as target).
  const { model } = await withLiveNode(() => connection.send('DOM.getBoxModel', { backendNodeId }, sessionId));
  const quad = model?.border;
  if (!Array.isArray(quad) || quad.length !== 8 || !quad.every(Number.isFinite)) throw new AgentToolRefusal('node_unavailable');
  const xs = [quad[0], quad[2], quad[4], quad[6]], ys = [quad[1], quad[3], quad[5], quad[7]];
  const left = Math.max(0, Math.floor(Math.min(...xs))), top = Math.max(0, Math.floor(Math.min(...ys)));
  const right = Math.min(page.width, Math.ceil(Math.max(...xs))), bottom = Math.min(page.height, Math.ceil(Math.max(...ys)));
  if (right <= left || bottom <= top) throw new AgentToolRefusal('node_off_screen');
  return { x: left, y: top, width: right - left, height: bottom - top };
}

async function assertFullPageProtection(host, tab, connection, sessionId) {
  if (host.protection.unknown) throw new AgentToolRefusal('full_page_protected');
  const targets = [...host.protection.targets, ...(host.browser.fillTargets?.values() ?? [])]
    .filter(target => target.kind === 'browser' && target.target_id === tab.target_id);
  if (!targets.length) return;
  try {
    // The shared collector visits every rendered field, including fields outside the viewport,
    // before clipping its masks. Password dots and retired fills need no pixel protection.
    await measurePageProtection(connection, sessionId, tab.target_id, { ...host.protection, targets }, {
      includeHidden: true, onPlainField() { throw new AgentToolRefusal('full_page_protected'); },
    });
  } catch { throw new AgentToolRefusal('full_page_protected'); }
}

async function fullPage(host, tab) {
  const { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id);
  await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
  const metrics = await connection.send('Page.getLayoutMetrics', {}, sessionId);
  const size = metrics.cssContentSize ?? metrics.contentSize;
  const width = Math.min(MAX_SIDE, Math.max(1, Math.ceil(size?.width)));
  const pageHeight = Math.max(1, Math.ceil(size?.height));
  const height = Math.min(MAX_SIDE, pageHeight);
  if (!Number.isFinite(width) || !Number.isFinite(pageHeight)) throw new AgentToolRefusal('image_too_large');
  const density = host.scales.get(tab.tab_id) ?? host.viewport().device_scale_factor;
  const viewport = host.pageSize(tab.tab_id), visual = metrics.cssVisualViewport;
  // At the document origin a whole page that fits the viewport already has complete trusted mask geometry.
  const protectedPage = host.protection.targets.some(target => target.target_id === tab.target_id) ||
    [...(host.browser.fillTargets?.values() ?? [])].some(target => target.target_id === tab.target_id);
  if (protectedPage && width <= viewport.width && pageHeight <= viewport.height && visual?.pageX === 0 && visual?.pageY === 0) {
    return { ...await host.displayScreenshot(tab, { x: 0, y: 0, width, height, scale: 1 / density }), page_height: pageHeight };
  }
  // One image pixel per CSS pixel, whatever the page density. A page whose PNG does not fit gets smaller encodings.
  const shoot = ({ format, quality, factor }) => connection.send('Page.captureScreenshot', {
    format, ...(quality ? { quality } : {}), captureBeyondViewport: true, optimizeForSpeed: true,
    clip: { x: 0, y: 0, width, height, scale: factor / density },
  }, sessionId);
  const taken = await host.sampleLane(tab).run('capture', () => host.browser.inputCapture.run(connection, sessionId, async () => {
    await assertFullPageProtection(host, tab, connection, sessionId);
    for (const attempt of FULL_PAGE_ATTEMPTS) {
      const shot = await shoot(attempt);
      if (typeof shot.data === 'string' && shot.data.length <= IMAGE_LIMIT) {
        await assertFullPageProtection(host, tab, connection, sessionId);
        return { ...attempt, data: shot.data };
      }
    }
    return null;
  }, { resumeFocus: false }));
  await assertCurrentDocument(connection, sessionId, tab.target_id, tab.document_id);
  if (!taken) throw new AgentToolRefusal('image_too_large');
  return { generation: host.generation, tab_id: tab.tab_id, document_id: tab.document_id, mime_type: `image/${taken.format}`, data_base64: taken.data,
    width: Math.round(width * taken.factor), height: Math.round(height * taken.factor), page_height: pageHeight,
    ...(taken.factor < 1 ? { scale: taken.factor } : {}), ...(pageHeight > height ? { truncated: true } : {}) };
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
