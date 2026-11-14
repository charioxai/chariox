import { checkedScene, sceneJSON } from './scene.js';
import { reconcileElements, restore, serializeAsJSON } from '@excalidraw/excalidraw';

// The backend and Excalidraw serialize the same scene with different key order
// and whitespace. Compare scene content so opening a view cannot bump its CAS.
const sceneKey = scene => JSON.stringify(scene, (_, value) =>
  value && typeof value === 'object' && !Array.isArray(value)
    ? Object.fromEntries(Object.entries(value).sort(([a], [b]) => a.localeCompare(b))) : value);

const POLL_MS = 1000;
const SAVE_ATTEMPTS = 4;

// An agent edits the saved scene through the App's backend while this window is open. The window
// saves its own edits against the version it last read; when that is refused, or when the change
// feed shows the backend moved, it merges the saved scene into the canvas with Excalidraw's own
// reconciliation (the newer version of each element wins; an element that the backend deleted is
// kept there as a marked element, so it is removed here too) and saves what is still its own.
export function persistence(chariox, api, loaded, feedHead) {
  let version = loaded.version;
  const normalized = scene => {
    const restored = restore(scene, null, null);
    return { restored, key: sceneKey(JSON.parse(serializeAsJSON(restored.elements, restored.appState, restored.files, 'local'))) };
  };
  let lastSaved = loaded.scene ? normalized(loaded.scene).key : null;
  let queue = Promise.resolve();
  let failure = null;
  let shownSelection = '';
  const enqueue = work => {
    const pending = queue.then(work);
    queue = pending.catch(() => {});
    return pending;
  };

  function merge(remote) {
    const { restored, key } = normalized(remote.scene);
    const merged = reconcileElements(api.getSceneElementsIncludingDeleted(), restored.elements, api.getAppState());
    api.addFiles(Object.values(restored.files));
    const selected = remote.scene.appState?.selectedElementIds;
    api.updateScene({ elements: merged, captureUpdate: 'NEVER' });
    // An agent's selection stays in the saved scene until the window saves; show each one once.
    const selection = selected ? JSON.stringify(Object.keys(selected).sort()) : '';
    if (selection && selection !== shownSelection) {
      shownSelection = selection;
      api.updateScene({ appState: { selectedElementIds: selected }, captureUpdate: 'NEVER' });
      api.scrollToContent(merged.filter(element => selected[element.id]), { fitToContent: true, animate: false });
    }
    version = remote.version;
    lastSaved = key;
  }

  const save = () => enqueue(async () => {
    for (let attempt = 1; ; attempt++) {
      const text = sceneJSON(api);
      const key = sceneKey(JSON.parse(text));
      if (key === lastSaved) return;
      if (failure) throw failure;
      // Validation failures can recover after a person makes the diagram smaller.
      const scene = checkedScene(text);
      try {
        version = (await chariox.call('save_scene', { scene, version })).version;
        lastSaved = key;
        return;
      } catch (error) {
        // A save is refused when the backend moved on: merge what it saved, then save what is left.
        const remote = await chariox.call('load_scene', {}).catch(() => null);
        if (remote && remote.version !== version && attempt < SAVE_ATTEMPTS) { merge(remote); continue; }
        failure = error;
        throw error;
      }
    }
  });

  const pull = () => enqueue(async () => {
    const remote = await chariox.call('load_scene', {});
    if (remote.version !== version) merge(remote);
  });

  let since = feedHead, stopped = false;
  (async () => {
    while (!stopped) {
      await new Promise(resolve => setTimeout(resolve, POLL_MS));
      try {
        const { latest } = await chariox.call('view_changes', { since });
        // The feed position moves only once the saved scene is merged, so a failed load is asked for again.
        if (latest !== since) { await pull(); since = latest; }
      } catch { /* the next tick asks again */ }
    }
  })();

  const filename = (name = 'diagram', extension = '.excalidraw') => name.endsWith(extension) ? name : name + extension;
  return {
    save,
    stop: () => { stopped = true; },
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
      // Keep the previous diagram in Excalidraw's native Undo history.
      api.addFiles(Object.values(scene.files));
      api.updateScene({ elements: scene.elements, appState: scene.appState, captureUpdate: 'IMMEDIATELY' });
      api.scrollToContent(undefined, { fitToContent: true, animate: false });
      await save();
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
  };
}
