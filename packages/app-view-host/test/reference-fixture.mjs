// Supplementary frontend-only adapter. This is never included in product assets
// and proves no kernel persistence, broker authority or provider operation.
import { createViewActions } from '../../../apps/kernel/slice-linux-docker/docker/browser-app-view-actions.mjs';

function backend() {
  let scene = null, version = null;
  return async (name, input) => {
    if (name === 'load_scene') return { scene, version };
    if (name === 'save_scene') { scene = input.scene; version = String(Number(version ?? 0) + 1); return { version }; }
    if (name === 'load_workbook') return { text: null, version: null, done: true };
    if (name === 'pick_file') return { grants: [] };
    throw new Error('Frontend feasibility fixture does not implement ' + name);
  };
}

export function referencePrelude() {
  return `const request=(${backend.toString()})();const sdk=(${createViewActions.toString()})(request);window.chariox={action:sdk.action,view:sdk.view,call:request};window.__charioxViewAgent=sdk.agent;\n`;
}
