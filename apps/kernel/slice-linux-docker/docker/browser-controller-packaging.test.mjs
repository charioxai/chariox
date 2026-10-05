// MP-08/MP-10/MP-11: verify the deployable controller's transitive module closure.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const root = path.dirname(fileURLToPath(import.meta.url));
test('MP-08/MP-10/MP-11 Docker controller bundle includes every local import', async () => {
  const dockerfile = await readFile(path.join(root,'Dockerfile'),'utf8');
  const pending = ['browser-controller.mjs'], seen = new Set();
  while (pending.length) {
    const name = pending.pop();
    if (seen.has(name)) continue;
    seen.add(name);
    assert.ok(dockerfile.split('\n').some(line => line.startsWith('COPY ')
      && line.endsWith(`/opt/chariox-slice/${name}`)),`MP-08/MP-10/MP-11 missing runtime module ${name}`);
    const source = await readFile(path.join(root,name),'utf8');
    for (const match of source.matchAll(/\b(?:from\s*|import\s*)["'](\.\/[^"']+\.mjs)["']/g)) {
      const next = path.normalize(path.join(path.dirname(name),match[1]));
      assert.ok(!next.startsWith('..') && !path.isAbsolute(next));
      pending.push(next);
    }
  }
  assert.ok(seen.has('browser-controller-interactions.mjs'));
  assert.ok(seen.has('browser-controller-geometry.mjs'));
});
