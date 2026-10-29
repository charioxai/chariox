import { checkedScene, sceneJSON } from './scene.js';
import { restore, serializeAsJSON } from '@excalidraw/excalidraw';

// The backend and Excalidraw serialize the same scene with different key order
// and whitespace. Compare scene content so opening a view cannot bump its CAS.
const sceneKey = scene => JSON.stringify(scene, (_, value) =>
  value && typeof value === 'object' && !Array.isArray(value)
    ? Object.fromEntries(Object.entries(value).sort(([a], [b]) => a.localeCompare(b))) : value);

export function persistence(chariox, api, loaded) {
  let version = loaded.version;
  const restored = loaded.scene ? restore(loaded.scene, null, null) : null;
  let lastSaved = restored ? sceneKey(JSON.parse(serializeAsJSON(restored.elements, restored.appState, restored.files, 'local'))) : null;
  let queue = Promise.resolve();
  let failure = null;
  let sequence = 0;
  let lastNotified = lastSaved;
  const notify = (text = sceneJSON(api)) => {
    const key = sceneKey(JSON.parse(text));
    if (key === lastNotified) return;
    lastNotified = key;
    chariox.view.changed({ sequence: ++sequence, persisted: !failure });
  };
  function save(candidate, caller = chariox) {
    const pending = queue.then(async () => {
      const text = candidate ?? sceneJSON(api);
      const key = sceneKey(JSON.parse(text));
      if (key === lastSaved) return;
      if (failure) throw failure;
      // Validation failures can recover after a person makes the diagram smaller.
      const scene = checkedScene(text);
      let result;
      try { result = await caller.call('save_scene', { scene, version }); }
      catch (error) { failure = error; throw error; }
      version = result.version;
      lastSaved = key;
      notify();
    });
    queue = pending.catch(() => {});
    return pending;
  }
  const filename = (name = 'diagram', extension = '.excalidraw') => name.endsWith(extension) ? name : name + extension;
  return {
    save,
    status: () => ({ saved: !failure, error: failure?.message ?? null }),
    async save_file({ name } = {}, caller) {
      // A private copy must remain possible after a concurrent-view conflict.
      await queue;
      const result = await caller.call('write_file', { name: filename(name ?? `diagram-${crypto.randomUUID()}`), text: sceneJSON(api), offer: name === undefined });
      api.setToast({ message: `Saved ${name === undefined ? 'copy' : 'private copy'}: ${result.name}${failure ? '; reload to recover automatic saving' : ''}` });
      return { ...result, automaticSaveError: failure?.message ?? null };
    },
    async open_file({ name } = {}, caller) {
      await queue;
      if (failure) throw failure;
      let request = { name: filename(name) };
      if (!name) {
        const { grants } = await caller.call('pick_file', {});
        if (!grants.length) return { cancelled: true };
        request = { grantId: grants[0] };
      }
      const result = await caller.call('read_file', request);
      const scene = checkedScene(JSON.stringify(result.scene));
      await save(JSON.stringify(scene), caller);
      // Keep the previous diagram in Excalidraw's native Undo history.
      api.addFiles(Object.values(scene.files));
      api.updateScene({ elements: scene.elements, appState: scene.appState, captureUpdate: 'IMMEDIATELY' });
      api.scrollToContent(undefined, { fitToContent: true, animate: false });
      return { name: result.name, elements: api.getSceneElements().length };
    },
    async export_file({ format, name = 'diagram' }, caller) {
      await queue;
      const { exportToBlob, exportToSvg } = await import('@excalidraw/excalidraw');
      const options = { elements: api.getSceneElements(), appState: { ...api.getAppState(), exportBackground: true, exportScale: 1 }, files: api.getFiles() };
      const request = { name: filename(name, format === 'json' ? '.excalidraw' : '.' + format), offer: true };
      if (format === 'json') request.text = sceneJSON(api);
      else if (format === 'svg') request.text = (await exportToSvg(options)).outerHTML;
      else if (format === 'png') {
        const blob = await exportToBlob({ ...options, mimeType: 'image/png' });
        if (!blob || blob.size > 144 * 1024) throw new Error('PNG exceeds the 144 KiB inline broker envelope; export SVG or JSON, or use a smaller diagram.');
        const bytes = new Uint8Array(await blob.arrayBuffer());
        let binary = '';
        for (let i = 0; i < bytes.length; i += 8192) binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
        request.base64 = btoa(binary);
      } else throw new Error('Choose png, svg or json.');
      const result = await caller.call('write_file', request);
      api.setToast({ message: `Exported ${result.name}` });
      return { ...result, automaticSaveError: failure?.message ?? null };
    },
    changed: notify,
  };
}
