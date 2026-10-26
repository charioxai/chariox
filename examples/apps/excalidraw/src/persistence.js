import { checkedScene, sceneJSON } from './scene.js';

export function persistence(chariox, api, loaded) {
  let version = loaded.version;
  let lastSaved = loaded.scene ? JSON.stringify(loaded.scene) : null;
  let queue = Promise.resolve();
  let failure = null;
  let sequence = 0;
  let lastNotified = lastSaved;
  const notify = (text = sceneJSON(api)) => {
    if (text === lastNotified) return;
    lastNotified = text;
    chariox.view.changed({ sequence: ++sequence, persisted: !failure });
  };
  function save(candidate) {
    const pending = queue.then(async () => {
      const text = candidate ?? sceneJSON(api);
      if (text === lastSaved) return;
      if (failure) throw failure;
      // Validation failures can recover after a person makes the diagram smaller.
      const scene = checkedScene(text);
      let result;
      try { result = await chariox.call('save_scene', { scene, version }); }
      catch (error) { failure = error; throw error; }
      version = result.version;
      lastSaved = text;
      notify();
    });
    queue = pending.catch(() => {});
    return pending;
  }
  const filename = (name = 'diagram', extension = '.excalidraw') => name.endsWith(extension) ? name : name + extension;
  return {
    save,
    status: () => ({ saved: !failure, error: failure?.message ?? null }),
    async save_file({ name } = {}) {
      // A private copy must remain possible after a concurrent-view conflict.
      await queue;
      const result = await chariox.call('write_file', { name: filename(name ?? `diagram-${crypto.randomUUID()}`), text: sceneJSON(api), offer: name === undefined });
      api.setToast({ message: `Saved ${name === undefined ? 'copy' : 'private copy'}: ${result.name}${failure ? '; reload to recover automatic saving' : ''}` });
      return { ...result, automaticSaveError: failure?.message ?? null };
    },
    async open_file({ name } = {}) {
      await queue;
      if (failure) throw failure;
      let request = { name: filename(name) };
      if (!name) {
        const { grants } = await chariox.call('pick_file', {});
        if (!grants.length) return { cancelled: true };
        request = { grantId: grants[0] };
      }
      const result = await chariox.call('read_file', request);
      const scene = checkedScene(JSON.stringify(result.scene));
      await save(JSON.stringify(scene));
      // Keep the previous diagram in Excalidraw's native Undo history.
      api.addFiles(Object.values(scene.files));
      api.updateScene({ elements: scene.elements, appState: scene.appState, captureUpdate: 'IMMEDIATELY' });
      api.scrollToContent(undefined, { fitToContent: true, animate: false });
      return { name: result.name, elements: api.getSceneElements().length };
    },
    async export_file({ format, name = 'diagram' }) {
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
      const result = await chariox.call('write_file', request);
      api.setToast({ message: `Exported ${result.name}` });
      return { ...result, automaticSaveError: failure?.message ?? null };
    },
    changed: notify,
  };
}
