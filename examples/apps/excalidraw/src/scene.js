import { serializeAsJSON } from '@excalidraw/excalidraw';

const MAX_BYTES = 192 * 1024;
const fail = message => { throw Object.assign(new Error(message), { code: 'INVALID_ARGUMENT' }); };
export const sceneJSON = api => serializeAsJSON(api.getSceneElements(), api.getAppState(), api.getFiles(), 'local');

// A deletion stays in the saved scene as long as in the backend, so a window that has not seen it
// yet learns of it when it merges; the backend forgets it after the same hour.
const TOMBSTONE_MS = 60 * 60 * 1000;
const byteLength = value => new TextEncoder().encode(JSON.stringify(value)).length;

/**
 * The scene to save: the live elements and the recent deletions, which a copy or an export leave
 * out. A deletion that does not fit the scene's size limit is not kept, so deleting a big
 * element never keeps the diagram from being saved.
 */
export function syncedSceneJSON(api, now = Date.now()) {
  const scene = JSON.parse(sceneJSON(api));
  const live = new Map(scene.elements.map(element => [element.id, element]));
  const recent = api.getSceneElementsIncludingDeleted()
    .filter(element => !live.has(element.id) && element.isDeleted && now - (element.updated ?? now) < TOMBSTONE_MS)
    .sort((a, b) => (b.updated ?? now) - (a.updated ?? now));
  let size = byteLength(scene);
  const kept = new Set();
  for (const element of recent) {
    const next = size + byteLength(element) + 1;
    if (next > MAX_BYTES) continue;
    size = next;
    kept.add(element.id);
  }
  scene.elements = api.getSceneElementsIncludingDeleted().flatMap(element =>
    live.has(element.id) ? [live.get(element.id)] : kept.has(element.id) ? [element] : []);
  return JSON.stringify(scene);
}

export function checkedScene(text) {
  if (new TextEncoder().encode(text).length > MAX_BYTES) fail('Diagram exceeds 192 KiB; split it into smaller files.');
  const scene = JSON.parse(text);
  if (scene.type !== 'excalidraw' || scene.version !== 2 || !Array.isArray(scene.elements)
      || !scene.appState || !scene.files) fail('Open an Excalidraw v2 JSON scene.');
  return scene;
}
