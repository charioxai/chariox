// The real drill must reject a kernel older than the checkpoint protocol before startup.
import assert from 'node:assert/strict';
import test from 'node:test';
import {mkdtemp, writeFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';

test('native checkpoint drill refuses protocol 636 before launching kernels', async () => {
    const scratch = await mkdtemp(join(tmpdir(), 'native-protocol-gate-'));
    try {
        const binary = join(scratch, 'kernel-version');
        await writeFile(binary, "#!/bin/sh\nprintf '636\\n'\n", {mode: 0o700});
        const env = {...process.env};
        for (const name of ['KERNEL', 'RELAY', 'TUI', 'TEST_BINARY']) env['CHARIOX_CHECKPOINT_' + name] = binary;
        env.CHARIOX_CHECKPOINT_STATE = join(scratch, 'state');
        env.CHARIOX_CHECKPOINT_EVIDENCE = join(scratch, 'evidence');
        env.CHARIOX_CHECKPOINT_SOURCE_PROFILE = join(scratch, 'empty-profile');
        const result = spawnSync(process.execPath, [new URL('./codex-native-checkpoint.mjs', import.meta.url).pathname], {env, encoding: 'utf8', timeout: 5000, killSignal: 'SIGKILL', maxBuffer: 1024 * 1024});
        assert.ok(result.stderr?.includes('Required native resume needs kernel protocol 637'), 'an older protocol must be refused by the native resume gate');
    } finally {
        await rm(scratch, {recursive: true, force: true});
    }
});
