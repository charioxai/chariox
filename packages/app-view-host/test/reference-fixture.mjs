// Supplementary frontend-only adapter. This is never included in product assets
// and proves no kernel persistence, broker authority or provider operation.
import { createViewActions } from '../../../apps/kernel/slice-linux-docker/docker/browser-app-view-actions.mjs';

function backend() {
  let scene = null, version = null, changes = 0;
  const request = async (name, input) => {
    if (name === 'load_scene') return { scene, version };
    if (name === 'save_scene') { scene = input.scene; version = String(Number(version ?? 0) + 1); changes++; return { version }; }
    if (name === 'view_changes') return { latest: changes, dropped: false, changes: [] };
    if (name === 'load_workbook') return { text: null, version: null, done: true };
    if (name === 'pick_file') return { grants: [] };
    throw new Error('Frontend feasibility fixture does not implement ' + name);
  };
  // What the page saved, for the checks that cannot ask a page that has no agent surface.
  request.scene = () => scene;
  return request;
}

// The bootstrap owns window.chariox (frozen, with the call only), so the stand-in SDK cannot replace it. The reference Apps
// read their SDK from the stand-in instead, until the page SDK (action, view) moves into the bootstrap.
export function adoptStandInSdk(script) {
  const prelude = `const request=(${backend.toString()})();const sdk=(${createViewActions.toString()})(request);globalThis.__standInChariox={action:sdk.action,view:sdk.view,call:request};globalThis.__standInBackend=request;window.__charioxViewAgent=sdk.agent;\n`;
  // A bare `chariox` would reach the frozen global too, so the App's code runs in a block that shadows it.
  return prelude + '{const chariox=globalThis.__standInChariox;\n' + script.replace(/\b(?:window|globalThis)\.chariox\b/g, 'globalThis.__standInChariox') + '\n}';
}
