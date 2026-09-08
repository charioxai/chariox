#!/usr/bin/env node
// Pinned client conformance only. No provider execution or live API account.
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { cp, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
const execute = promisify(execFile);
const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const parent = join(homedir(), '.chariox/dev/apps-phase1/fetch-client-tests');
await mkdir(parent, { recursive: true, mode: 0o700 });
const scratch = await mkdtemp(join(parent, 'run-'));
const environment = { PATH: process.env.PATH, HOME: process.env.HOME, LANG: 'C', LC_ALL: 'C', npm_config_update_notifier: 'false' };
try {
  for (const name of ['package.json', 'package-lock.json']) {
    await cp(join(repository, 'packages/app-sdk/test/client-fixtures', name), join(scratch, name));
  }
  // Separate empty npm config files avoid ambient authentication/registries.
  for (const name of ['user.npmrc', 'global.npmrc']) await writeFile(join(scratch, name), '', { mode: 0o600 });
  const install = await execute('npm', ['ci', '--ignore-scripts', '--no-audit', '--no-fund',
    '--registry=https://registry.npmjs.org', `--userconfig=${join(scratch, 'user.npmrc')}`,
    `--globalconfig=${join(scratch, 'global.npmrc')}`, `--cache=${join(scratch, 'cache')}`,
    `--prefix=${scratch}`], { cwd: scratch, env: environment, timeout: 60_000, maxBuffer: 1024 * 1024 });
  process.stdout.write(install.stdout);
  // node:test also runs when its fixed file is the direct entrypoint, avoiding
  // a child test-runner process that could outlive this bounded command.
  const checked = await execute(process.execPath, ['--max-old-space-size=256',
    join(repository, 'packages/app-sdk/test/client-fixtures/clients.test.mjs')], {
    cwd: scratch, timeout: 30_000, maxBuffer: 1024 * 1024,
    env: { ...environment, CHARIOX_FETCH_CLIENT_FIXTURES: scratch },
  });
  process.stdout.write(checked.stdout);
} catch (error) {
  if (error.stdout) process.stdout.write(error.stdout);
  if (error.stderr) process.stderr.write(error.stderr);
  process.exitCode = 1;
} finally { await rm(scratch, { recursive: true, force: true }); }
