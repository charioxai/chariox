import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

// Use only bytes named by the native kernel's embedding manifest, outside the
// checkout. Importing the source entry would hide a missing transitive asset.
test('default embedded browser entry loads its entire static dependency closure and answers health', async () => {
  const manifest = new URL('../src/runtime/kernel_browser_assets.rs', import.meta.url);
  const source = await readFile(manifest, 'utf8');
  const root = await mkdtemp(path.join(tmpdir(), 'chariox-embedded-browser-'));
  try {
    const names = new Set();
    for (const match of source.matchAll(/"([^"]+)",\s*include_bytes!\("([^"]+)"\)/g)) {
      names.add(match[1]);
      await writeFile(path.join(root, match[1]), await readFile(new URL(match[2], manifest)));
    }
    assert(names.has('kernel-browser-host.mjs'));
    const pending = ['kernel-browser-host.mjs'], seen = new Set();
    while (pending.length) {
      const name = pending.pop();
      if (seen.has(name)) continue;
      seen.add(name);
      for (const match of (await readFile(path.join(root, name), 'utf8')).matchAll(/\b(?:from|import)\s*["'](\.[^"']+)["']/g)) {
        const dependency = path.posix.normalize(path.posix.join(path.posix.dirname(name), match[1]));
        assert(names.has(dependency), `${name} imports unembedded ${dependency}`);
        pending.push(dependency);
      }
    }
    const env = { PATH: process.env.PATH };
    const result = spawnSync(process.execPath, [path.join(root, 'kernel-browser-host.mjs'), 'stdio', root], {
      cwd: root, env, encoding: 'utf8', timeout: 10000,
      input: '{"id":1,"method":"health","params":{}}\n',
    });
    assert.equal(result.status, 0, result.stderr);
    const health = JSON.parse(result.stdout.trim());
    assert.equal(health.ok, true);
    assert.equal(health.result.state, 'ready');
    assert(seen.size > 20, 'Check the shared controller dependencies as well as the host');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
