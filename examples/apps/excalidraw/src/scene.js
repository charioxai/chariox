import { serializeAsJSON } from '@excalidraw/excalidraw';

const MAX_BYTES = 192 * 1024;
const fail = message => { throw Object.assign(new Error(message), { code: 'INVALID_ARGUMENT' }); };
export const sceneJSON = api => serializeAsJSON(api.getSceneElements(), api.getAppState(), api.getFiles(), 'local');

// A deletion stays in the saved scene as long as in the backend, so a window that has not seen it
// yet learns of it when it merges; the backend forgets it after the same hour.
const TOMBSTONE_MS = 60 * 60 * 1000;
// What a window needs to remove an element it still shows: which element, which version, and where
// it sorts. The rest of the element (text, points, embedded data) is not kept, so a deletion is
// a few hundred bytes however big the element was.
const TOMBSTONE_FIELDS = ['id', 'type', 'x', 'y', 'width', 'height', 'angle', 'index', 'seed', 'groupIds', 'frameId',
  'version', 'versionNonce', 'updated', 'isDeleted'];
// Excalidraw restores deleted elements too, and reads some fields of each type before anything else:
// an element with fewer than two points is dropped as invisible (its deletion would be lost), and a
// text without a string or a line height throws. A record keeps the least that restores unchanged.
const typeFields = element => {
  switch (element.type) {
    case 'arrow': case 'line': case 'freedraw':
      return { points: [[0, 0], [element.width ?? 0, element.height ?? 0]] };
    case 'text':
      return { text: '', fontSize: element.fontSize, fontFamily: element.fontFamily, lineHeight: element.lineHeight };
    default:
      return {};
  }
};
const tombstone = element => ({
  ...Object.fromEntries(TOMBSTONE_FIELDS.filter(field => field in element).map(field => [field, element[field]])),
  ...typeFields(element),
});

/**
 * The scene to save: the live elements and the deletions of the last hour as compact records,
 * which a copy or an export leave out. A fresh deletion is never dropped to fit the size limit:
 * when the scene is still too big, saving it fails with that limit.
 */
export function syncedSceneJSON(api, now = Date.now()) {
  const scene = JSON.parse(sceneJSON(api));
  const live = new Map(scene.elements.map(element => [element.id, element]));
  scene.elements = api.getSceneElementsIncludingDeleted().flatMap(element => {
    if (live.has(element.id)) return [live.get(element.id)];
    return element.isDeleted && now - (element.updated ?? now) < TOMBSTONE_MS ? [tombstone(element)] : [];
  });
  return JSON.stringify(scene);
}

/**
 * The elements of a diagram that is opened over the shown one. Excalidraw's `updateScene` replaces
 * the array, and a window that still shows the old diagram keeps every element that the saved scene
 * does not mention, so it would show both and could save the old one back. Each shown element the new
 * diagram lacks therefore stays as a deleted one, and one both have gets a version above both, so the
 * other window removes and replaces by the same rule as for any other edit.
 */
export function replacement(shown, opened, now = Date.now()) {
  const nonce = () => Math.floor(Math.random() * 2 ** 31);
  const before = new Map(shown.map(element => [element.id, element]));
  const incoming = new Set(opened.map(element => element.id));
  const elements = opened.map(element => {
    const old = before.get(element.id);
    return old ? { ...element, version: Math.max(old.version, element.version ?? 0) + 1, versionNonce: nonce(), updated: now } : element;
  });
  const removed = shown.filter(element => !incoming.has(element.id))
    .map(element => element.isDeleted ? element : { ...element, isDeleted: true, version: element.version + 1, versionNonce: nonce(), updated: now });
  return [...elements, ...removed];
}

export function checkedScene(text) {
  if (new TextEncoder().encode(text).length > MAX_BYTES) fail('Diagram exceeds 192 KiB; split it into smaller files.');
  const scene = JSON.parse(text);
  if (scene.type !== 'excalidraw' || scene.version !== 2 || !Array.isArray(scene.elements)
      || !scene.appState || !scene.files) fail('Open an Excalidraw v2 JSON scene.');
  return scene;
}
