// Writes the signed declarations from src/catalog.mjs: tools, UI actions and the manifest's REST routes.
// usage: node scripts/generate.mjs            (rewrite the committed files)
//        node scripts/generate.mjs --check    (fail when a committed file differs)
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { eventRoutingCapability, eventsDocument, networkCapability, tools, toolsDocument, uiActionsDocument, workspaceCapability } from '../src/catalog.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const pretty = value => JSON.stringify(value, null, 2) + '\n';

export async function outputs() {
  const manifest = JSON.parse(await readFile(join(root, 'app.json'), 'utf8'));
  manifest.capabilities.network = [networkCapability()];
  manifest.capabilities.eventRouting = eventRoutingCapability();
  manifest.capabilities.workspace = workspaceCapability();
  manifest.idempotentTools = [...tools.filter(tool => tool.effect === 'read').map(tool => tool.name), 'view_account', 'view_actions', 'view_changes', 'view_state'].sort();
  return {
    'app.json': pretty(manifest),
    'bundle/schemas/events.json': pretty(eventsDocument()),
    'bundle/schemas/tools.json': pretty(toolsDocument()),
    'bundle/schemas/ui-actions.json': pretty(uiActionsDocument()),
  };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const check = process.argv.includes('--check');
  let stale = 0;
  for (const [file, text] of Object.entries(await outputs())) {
    const path = join(root, file);
    const current = await readFile(path, 'utf8').catch(() => null);
    if (current === text) continue;
    if (check) { console.error(`${file} differs from src/catalog.mjs; run node scripts/generate.mjs`); stale++; } else await writeFile(path, text);
  }
  if (stale) process.exit(1);
}
