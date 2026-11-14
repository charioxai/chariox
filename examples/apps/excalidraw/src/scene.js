import { serializeAsJSON } from '@excalidraw/excalidraw';

const MAX_BYTES = 192 * 1024;
const fail = message => { throw Object.assign(new Error(message), { code: 'INVALID_ARGUMENT' }); };
export const sceneJSON = api => serializeAsJSON(api.getSceneElements(), api.getAppState(), api.getFiles(), 'local');

export function checkedScene(text) {
  if (new TextEncoder().encode(text).length > MAX_BYTES) fail('Diagram exceeds 192 KiB; split it into smaller files.');
  const scene = JSON.parse(text);
  if (scene.type !== 'excalidraw' || scene.version !== 2 || !Array.isArray(scene.elements)
      || !scene.appState || !scene.files) fail('Open an Excalidraw v2 JSON scene.');
  return scene;
}
