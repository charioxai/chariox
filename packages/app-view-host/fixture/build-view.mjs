// Writes the fixture's self-contained UI entry, byte for byte what the client verifier expects.
// usage: node build-view.mjs <output directory outside the checkout>
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { isAbsolute, join } from 'node:path';
import { encodeView } from '../bundle.mjs';

const output = process.argv[2];
if (!output || !isAbsolute(output)) throw new Error('usage: build-view.mjs <absolute output directory>');
const script = await readFile(new URL('./view.js', import.meta.url), 'utf8');
const style = 'body{font:14px system-ui,sans-serif;margin:0;padding:12px;color:#111;background:#fff}form{display:flex;gap:8px}input{flex:1;padding:4px 6px}#status{min-height:1.2em;margin:8px 0;color:#444}#status[data-state=refused],#status[data-state=failed]{color:#a00}ul{padding-left:20px}';
await mkdir(output, { recursive: true });
await writeFile(join(output, 'index.html'), encodeView({ schema: 'chariox.app-view-source.v1', rootId: 'app', script, style }));
