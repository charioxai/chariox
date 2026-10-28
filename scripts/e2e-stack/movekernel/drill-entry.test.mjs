// MP-08 / MP-09 / MP-10 / MP-11: documented native drills load before starting runtime state.
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import test from 'node:test';

for (const script of ['return-native-conflict.mjs', 'durable-move.mjs']) {
    test(script + ' loads its helpers without starting kernels', () => {
        const result = spawnSync(process.execPath, [new URL(script, import.meta.url).pathname, '--help'], {
            env: {PATH: process.env.PATH, NODE_OPTIONS: '--max-old-space-size=4096'},
            encoding: 'utf8', timeout: 10000, maxBuffer: 1024 * 1024,
        });
        assert.equal(result.status, 0, result.stderr);
        assert.match(result.stdout, /CHARIOX_WORKSPACE_KERNEL/);
    });
}
