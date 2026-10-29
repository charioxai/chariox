import React from 'react';
import { createRoot } from 'react-dom/client';
import { Excalidraw, MainMenu } from '@excalidraw/excalidraw';
import '@excalidraw/excalidraw/index.css';
import declarations from '../schemas/ui-actions.json';
import { diagramActions, sceneJSON, summary } from './scene.js';
import { persistence } from './persistence.js';

const chariox = window.chariox;
if (!chariox?.action || !chariox?.view || !chariox?.call) throw new Error('Open this App in a Chariox kernel with WASM and UI action support.');
let api, storage, actions, timer, previousScene;
let actionQueue = Promise.resolve();
const loaded = await chariox.call('load_scene', {});
let automaticSaveError;
const report = error => {
  const message = error.message ?? String(error);
  if (!api) return;
  api.setToast({ message, closable: true, duration: Infinity });
};
const reportAutomaticSave = error => {
  const message = error.message ?? String(error);
  if (automaticSaveError === message && api.getAppState().toast?.message === message) return;
  automaticSaveError = message;
  report(error);
};
const invoke = name => () => actions[name](name === 'export_file' ? { format: 'png' } : {}).catch(report);
const ready = next => {
  if (api) return;
  api = next;
  storage = persistence(chariox, api, loaded);
  actions = Object.fromEntries(declarations.uiActions.map(declaration => [declaration.name,
    chariox.action(declaration, (input, caller) => {
      const result = actionQueue.then(() => {
        const handlers = { ...diagramActions(api, candidate => storage.save(candidate, caller)), ...storage };
        return handlers[declaration.name](input, caller);
      });
      actionQueue = result.catch(() => {});
      return result;
    }),
  ]));
  // Excalidraw installs its default actions after calling excalidrawAPI.
  queueMicrotask(() => { for (const [name, handler, key, shift] of [
    ['loadScene', 'open_file', 'o', false],
    ['saveToActiveFile', 'save_file', 's', false],
    ['saveFileToDisk', 'save_file', 's', true],
  ]) api.registerAction({
    name, label: name, trackEvent: false,
    keyTest: event => (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === key && event.shiftKey === shift,
    perform: async () => { try { await actions[handler]({}); } catch (error) { report(error); } return false; },
  });
  });
  chariox.view.state(() => ({ ...summary(api), persistence: storage.status() }));
};

// A dropped browser file must be imported by a kernel grant as well.
document.addEventListener('drop', event => {
  if (!event.dataTransfer?.files.length) return;
  event.preventDefault(); event.stopPropagation();
  if ([...event.dataTransfer.files].some(file => !/\.(excalidraw|json)$/i.test(file.name))) {
    report(new Error('Open an Excalidraw or JSON diagram. Image insertion is not supported by this App.'));
    return;
  }
  api.setToast({ message: 'Choose the dropped diagram in the file picker.' });
  actions.open_file({}).catch(report);
}, true);
document.addEventListener('paste', event => {
  if (!event.clipboardData?.files.length) return;
  event.preventDefault(); event.stopPropagation();
  report(new Error('Image insertion is not supported. Use Open for an Excalidraw or JSON diagram.'));
}, true);

createRoot(document.getElementById('root')).render(
  <Excalidraw excalidrawAPI={ready} initialData={loaded.scene ?? { appState: { currentItemFontFamily: 1 } }}
    UIOptions={{ canvasActions: { export: false, saveAsImage: false }, tools: { image: false } }}
    onPaste={data => {
      if (!data.elements?.some(element => element.type === 'image') && !data.files && !data.mixedContent?.some(item => item.type === 'imageUrl')) return true;
      report(new Error('Image insertion is not supported by this App.'));
      return false;
    }}
    onChange={() => {
      if (!storage) return;
      const text = sceneJSON(api);
      // Notices, selection and other UI state do not change the saved scene.
      if (text === previousScene) return;
      storage.changed(text);
      previousScene = text;
      clearTimeout(timer);
      timer = setTimeout(() => storage.save().then(() => { automaticSaveError = undefined; }).catch(reportAutomaticSave), 300);
    }}>
    <MainMenu>
      <MainMenu.Item onSelect={invoke('open_file')}>Open</MainMenu.Item>
      <MainMenu.Item onSelect={invoke('save_file')}>Save a copy</MainMenu.Item>
      <MainMenu.Item onSelect={invoke('export_file')}>Export image</MainMenu.Item>
      <MainMenu.Item onSelect={() => actions.export_file({ format: 'svg' }).catch(report)}>Export SVG</MainMenu.Item>
      <MainMenu.Item onSelect={() => actions.export_file({ format: 'json' }).catch(report)}>Export JSON</MainMenu.Item>
      <MainMenu.DefaultItems.ClearCanvas />
      <MainMenu.Separator />
      <MainMenu.DefaultItems.ChangeCanvasBackground />
    </MainMenu>
  </Excalidraw>,
);
