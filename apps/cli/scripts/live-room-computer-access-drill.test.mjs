import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const drill = fileURLToPath(new URL('./live-room-computer-access-drill.mjs', import.meta.url));
const which = tool => process.env.PATH.split(path.delimiter).map(dir => path.join(dir, tool)).find(file => existsSync(file)) ?? assert.fail(`missing ${tool}`);

for (const seam of ['kernel', 'tui', 'platform', 'resources', 'pty']) {
  test(`MP-08 / MP-10 / MP-11: ${seam} preflight failure reports and removes disposable state`, {
    skip: process.platform !== 'linux' && seam !== 'platform',
  }, async () => {
    const sandbox = await mkdtemp(path.join(os.tmpdir(), 'chariox-room-access-preflight-test-'));
    try {
      const tmp = path.join(sandbox, 'tmp'), evidence = path.join(sandbox, 'evidence');
      await mkdir(tmp);
      const missing = path.join(sandbox, 'unbuilt-binary');
      let toolPath = process.env.PATH;
      if (seam === 'resources' || seam === 'pty') {
        // Tools resolve from the parent PATH; pty fails before the host-dependent resource floor.
        toolPath = path.join(sandbox, 'bin');
        await mkdir(toolPath);
        for (const tool of seam === 'resources' ? ['git', 'script'] : ['git']) await symlink(which(tool), path.join(toolPath, tool));
      }
      const nodeArgs = [];
      if (seam === 'platform') {
        const preload = path.join(sandbox, 'platform.mjs');
        await writeFile(preload, "Object.defineProperty(process, 'platform', { value: 'darwin' });\n");
        nodeArgs.push('--import', preload);
      }
      const child = spawnSync(process.execPath, [...nodeArgs, drill, '--evidence', evidence,
        '--kernel', seam === 'kernel' ? missing : process.execPath,
        '--tui', seam === 'tui' ? missing : process.execPath], {
        env: { PATH: toolPath, LANG: 'C.UTF-8', TMPDIR: tmp },
        encoding: 'utf8', timeout: 15000,
      });
      assert.equal(child.error, undefined);
      assert.equal(child.signal, null);
      assert.equal(child.status, 1);
      assert.deepEqual(await readdir(tmp), [], 'preflight must not leak its disposable root');
      const report = JSON.parse(await readFile(path.join(evidence, 'result.json'), 'utf8'));
      assert.equal(report.status, 'FAIL');
      const failure = {
        platform: /Linux-only/, resources: /spawnSync awk ENOENT/, pty: /spawnSync script ENOENT/,
        kernel: /ENOENT.*unbuilt-binary/, tui: /ENOENT.*unbuilt-binary/,
      };
      assert.match(report.failure, failure[seam]);
      assert.deepEqual(report.items, ['MP-08', 'MP-10', 'MP-11']);
      assert.match(report.source_commit, /^[a-f0-9]{40}$/);
      assert.match(report.cleanup, /disposable state removed/i);
      assert.ok(report.finished_at);
      assert.match(child.stdout, /FAIL room computer bulk access drill/);
      assert.equal(child.stderr, '');
    } finally {
      await rm(sandbox, { recursive: true, force: true });
    }
  });
}
