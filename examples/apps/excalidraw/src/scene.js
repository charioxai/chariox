import { convertToExcalidrawElements, newElementWith, serializeAsJSON } from '@excalidraw/excalidraw';

const MAX_BYTES = 192 * 1024;
const fail = message => { throw Object.assign(new Error(message), { code: 'INVALID_ARGUMENT' }); };
export const sceneJSON = api => serializeAsJSON(api.getSceneElements(), api.getAppState(), api.getFiles(), 'local');

function boundary(shape, towards) {
  const center = { x: shape.x + shape.width / 2, y: shape.y + shape.height / 2 };
  const angle = shape.angle ?? 0, cos = Math.cos(angle), sin = Math.sin(angle);
  const vx = towards.x - center.x, vy = towards.y - center.y;
  const dx = vx * cos + vy * sin, dy = -vx * sin + vy * cos;
  const rx = shape.width / 2, ry = shape.height / 2;
  const length = Math.hypot(dx, dy);
  if (!length) return center;
  const scale = shape.type === 'ellipse' ? 1 / Math.hypot(dx / rx, dy / ry)
    : shape.type === 'diamond' ? 1 / (Math.abs(dx) / rx + Math.abs(dy) / ry)
    : Math.min(rx / Math.abs(dx || Number.MIN_VALUE), ry / Math.abs(dy || Number.MIN_VALUE));
  return { x: center.x + vx * (scale + 5 / length), y: center.y + vy * (scale + 5 / length) };
}

export function checkedScene(text) {
  if (new TextEncoder().encode(text).length > MAX_BYTES) fail('Diagram exceeds 192 KiB; split it into smaller files.');
  const scene = JSON.parse(text);
  if (scene.type !== 'excalidraw' || scene.version !== 2 || !Array.isArray(scene.elements)
      || !scene.appState || !scene.files) fail('Open an Excalidraw v2 JSON scene.');
  return scene;
}

export function summary(api, { offset = 0, limit = 50 } = {}) {
  const all = api.getSceneElements();
  const candidates = all.slice(offset, offset + limit).map(e => ({
    id: e.id, type: e.type, x: e.x, y: e.y, width: e.width, height: e.height,
    angle: e.angle, groups: e.groupIds.slice(0, 16), groupsTruncated: e.groupIds.length > 16, containerId: e.containerId ?? null,
    ...(e.type === 'text' ? { text: e.text.slice(0, 500), textTruncated: e.text.length > 500 } : {}),
    ...(e.points ? { points: e.points.slice(0, 4), pointsTruncated: e.points.length > 4 } : {}),
    ...(e.type === 'arrow' ? { from: e.startBinding?.elementId ?? null, to: e.endBinding?.elementId ?? null } : {}),
  }));
  const elements = [];
  let bytes = 0;
  for (const candidate of candidates) {
    const size = new TextEncoder().encode(JSON.stringify(candidate)).length;
    if (bytes + size > 48 * 1024) break;
    elements.push(candidate); bytes += size;
  }
  return { total: all.length, offset, nextOffset: offset + elements.length < all.length ? offset + elements.length : null,
    selected: Object.keys(api.getAppState().selectedElementIds).slice(0, 100), elements };
}

export function diagramActions(api, persist) {
  const all = () => api.getSceneElements();
  const selected = ids => {
    const elements = all().filter(e => ids.includes(e.id));
    if (elements.length !== ids.length) fail('Every ID must identify an existing element. Read scene_summary first.');
    return elements;
  };
  async function update(elements, selectedElementIds) {
    // The canvas and agent paths share Excalidraw's scene store and undo history.
    const candidate = serializeAsJSON(elements.filter(e => !e.isDeleted), api.getAppState(), api.getFiles(), 'local');
    checkedScene(candidate);
    const previous = sceneJSON(api);
    await persist(candidate);
    if (sceneJSON(api) !== previous) {
      await persist(sceneJSON(api));
      fail('The canvas changed while this action was saving. Read the scene and retry.');
    }
    api.updateScene({ elements, ...(selectedElementIds ? { appState: { selectedElementIds } } : {}), captureUpdate: 'IMMEDIATELY' });
    return summary(api);
  }
  return {
    async add_elements({ elements }) {
      const known = new Set(all().map(e => e.id));
      const skeletons = elements.map(e => {
        if (e.id && known.has(e.id)) fail(`Element ID already exists: ${e.id}`);
        if (e.id) known.add(e.id);
        const base = { ...e, fontFamily: 1, ...(e.label ? { label: { text: e.label, fontFamily: 1 } } : {}) };
        if (e.type === 'text') return { ...base, text: e.text ?? e.label ?? '', fontSize: 20 };
        if (e.type === 'arrow') return { ...base, width: undefined, height: undefined,
          x: e.x, y: e.y, points: [[0, 0], [(e.endX ?? e.x + 160) - e.x, (e.endY ?? e.y) - e.y]] };
        return { ...base, width: e.width ?? 160, height: e.height ?? 90 };
      });
      const additions = convertToExcalidrawElements(skeletons, { regenerateIds: false });
      return update([...all(), ...additions]);
    },
    async update_elements({ ids, dx = 0, dy = 0, width, height }) {
      const targets = selected(ids);
      if (targets.some(e => !['rectangle', 'ellipse', 'diamond', 'text'].includes(e.type))) fail('Move/resize supports shapes and text; redraw arrows with connect_elements.');
      if ((width !== undefined || height !== undefined) && targets.some(e => e.type === 'text')) fail('Resize the text container, or move the text.');
      const changed = new Map(targets.map(e => [e.id, newElementWith(e, { x: e.x + dx, y: e.y + dy,
        ...(width === undefined ? {} : { width }), ...(height === undefined ? {} : { height }) })]));
      for (const e of all()) if (e.type === 'text' && e.containerId && changed.has(e.containerId)) {
        const old = all().find(t => t.id === e.containerId), next = changed.get(e.containerId);
        changed.set(e.id, newElementWith(e, { x: e.x + next.x - old.x + (next.width - old.width) / 2,
          y: e.y + next.y - old.y + (next.height - old.height) / 2 }));
      }
      for (const e of all()) if (e.type === 'arrow' && (changed.has(e.startBinding?.elementId) || changed.has(e.endBinding?.elementId))) {
        const oldFrom = all().find(t => t.id === e.startBinding?.elementId);
        const oldTo = all().find(t => t.id === e.endBinding?.elementId);
        const from = changed.get(oldFrom?.id) ?? oldFrom, to = changed.get(oldTo?.id) ?? oldTo;
        const oldStart = { x: e.x + e.points[0][0], y: e.y + e.points[0][1] };
        const last = e.points.at(-1), oldEnd = { x: e.x + last[0], y: e.y + last[1] };
        const fromCenter = from ? { x: from.x + from.width / 2, y: from.y + from.height / 2 } : oldStart;
        const toCenter = to ? { x: to.x + to.width / 2, y: to.y + to.height / 2 } : oldEnd;
        const { x, y } = from ? boundary(from, toCenter) : oldStart;
        const { x: endX, y: endY } = to ? boundary(to, fromCenter) : oldEnd;
        const points = e.points.map(([px, py], i) => {
          const t = i / (e.points.length - 1);
          return [e.x + px + (x - oldStart.x) * (1 - t) + (endX - oldEnd.x) * t - x,
            e.y + py + (y - oldStart.y) * (1 - t) + (endY - oldEnd.y) * t - y];
        });
        const next = newElementWith(e, { x, y, width: Math.max(...points.map(p => p[0])) - Math.min(...points.map(p => p[0])),
          height: Math.max(...points.map(p => p[1])) - Math.min(...points.map(p => p[1])), points });
        changed.set(e.id, next);
        for (const label of all()) if (label.type === 'text' && label.containerId === e.id) {
          changed.set(label.id, newElementWith(label, { x: (x + endX - label.width) / 2, y: (y + endY - label.height) / 2 }));
        }
      }
      return update(all().map(e => changed.get(e.id) ?? e));
    },
    async delete_elements({ ids }) {
      selected(ids);
      const removed = new Set(ids);
      for (const e of all()) if (removed.has(e.containerId)) removed.add(e.id);
      const elements = all().map(e => removed.has(e.id) ? newElementWith(e, { isDeleted: true }) : newElementWith(e, {
        boundElements: e.boundElements?.filter(b => !removed.has(b.id)) ?? null,
        ...(e.type === 'arrow' ? { startBinding: removed.has(e.startBinding?.elementId) ? null : e.startBinding,
          endBinding: removed.has(e.endBinding?.elementId) ? null : e.endBinding } : {}),
      }));
      return update(elements, {});
    },
    async group_elements({ ids }) {
      selected(ids);
      const group = crypto.randomUUID();
      return update(all().map(e => ids.includes(e.id) || ids.includes(e.containerId)
        ? newElementWith(e, { groupIds: [...e.groupIds, group] }) : e));
    },
    async connect_elements({ from, to, label }) {
      if (from === to) fail('Choose two distinct shapes.');
      const targets = selected([from, to]);
      if (targets.some(e => !['rectangle', 'ellipse', 'diamond'].includes(e.type))) fail('Connect rectangle, ellipse or diamond shapes.');
      const id = crypto.randomUUID();
      const connected = convertToExcalidrawElements([...targets.map(e => structuredClone(e)), { id, type: 'arrow', x: 0, y: 0,
        start: { id: from }, end: { id: to }, ...(label ? { label: { text: label, fontFamily: 1 } } : {}) }], { regenerateIds: false });
      const replacements = new Map(connected.map(e => [e.id, e]));
      return update([...all().map(e => replacements.get(e.id) ?? e), ...connected.filter(e => !targets.some(t => t.id === e.id))]);
    },
    async select_elements({ ids }) {
      selected(ids);
      api.updateScene({ appState: { selectedElementIds: Object.fromEntries(ids.map(id => [id, true])) } });
      api.scrollToContent(selected(ids), { fitToContent: true, animate: false });
      return summary(api);
    },
    scene_summary: input => summary(api, input),
  };
}
