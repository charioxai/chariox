import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { dockerAdmissionSetupCommand } from './app-runtime-local-release.mjs';

test('the macOS developer receipt names the boot installer and quotes the checkout path', () => {
  const command = dockerAdmissionSetupCommand("/tmp/Chariox owner's checkout");
  // Parse the printed command without executing sudo or touching any host files.
  const parsed = spawnSync('sh', ['-c', `set -- ${command}; printf '%s\\n' "$@"`], { encoding: 'utf8' });
  assert.equal(parsed.status, 0, parsed.stderr);
  assert.deepEqual(parsed.stdout.trim().split('\n'), ['sudo', '/usr/bin/python3',
    "/tmp/Chariox owner's checkout/deploy/local-macos/install-docker-admission-locks.py"]);
});
