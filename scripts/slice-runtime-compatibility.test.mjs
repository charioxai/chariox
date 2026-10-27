// MP-08/MP-10/MP-11: supplementary build-time ABI admission checks.
import assert from 'node:assert/strict';
import { chmod, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const script = fileURLToPath(new URL('../apps/kernel/slice-linux-docker/docker/check-runtime.sh', import.meta.url));
for (const rejected of [null, 'chariox-kernel', 'chariox-relay']) {
  test(`slice runtime compatibility probe ${rejected ?? 'accepts loadable binaries'}`, async t => {
    const root = await mkdtemp(join(tmpdir(), 'chariox-slice-abi-'));
    t.after(() => rm(root, { recursive: true, force: true }));
    for (const name of ['chariox-kernel', 'chariox-relay']) {
      const option = name === 'chariox-kernel' ? '--print-local-daemon-protocol-version' : '--help';
      const path = join(root, name);
      await writeFile(path, `#!/bin/sh\n[ "$1" = '${option}' ] || exit 91\nexit ${name === rejected ? 127 : 0}\n`);
      await chmod(path, 0o755);
    }
    const result = spawnSync('sh', [script, root], { encoding: 'utf8' });
    assert.equal(result.status, rejected ? 1 : 0, result.stderr);
    if (rejected) assert.match(result.stderr, /Rebuild.*without CHARIOX_PREBUILT_RUNTIME=1/);
  });
}

test('slice image checks the binaries inside the final runtime', async () => {
  const dockerfile = await readFile(new URL('../apps/kernel/slice-linux-docker/docker/Dockerfile', import.meta.url), 'utf8');
  assert.ok(dockerfile.indexOf('RUN /opt/chariox-slice/check-runtime.sh') > dockerfile.indexOf('COPY --from=rust-builder --chown=slice:slice /opt/chariox-runtime-bin/chariox-relay'));
});
