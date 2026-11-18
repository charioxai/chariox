// The Excalidraw page as the self-contained client view that a window carries as the App's UI entry.
// usage: node scripts/build-client-view.mjs <absolute output directory outside the checkout>
// Writes <output>/index.html (the view) next to <output>/build (the bundle it was made from).
import { spawnSync } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { referenceSource } from '../../../../packages/app-view-host/build-reference.mjs';
import { encodeView } from '../../../../packages/app-view-host/bundle.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const output = process.argv[2];
if (!output || !isAbsolute(output) || !relative(root, output).startsWith('..')) {
  throw Error('usage: node scripts/build-client-view.mjs /absolute/output/directory (outside the checkout)');
}
const build = join(output, 'build');
await mkdir(output, { recursive: true });
const built = spawnSync(process.execPath, [join(root, 'scripts/build-view.mjs'), build, '--client-view'], { cwd: root, stdio: 'inherit' });
if (built.status !== 0) throw Error('The client view build failed');
await writeFile(join(output, 'index.html'), encodeView(await referenceSource('excalidraw', build)));
