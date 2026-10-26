// Linux integration of union protocol 461/92. Use a normal Unix user,
// an already built kernel lib-test binary and an explicit sandboxed Chromium.
import { spawnSync } from 'node:child_process';
import { chmodSync, closeSync, copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync, openSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

if (process.platform !== 'linux' || process.getuid?.() === 0) throw new Error('Run this drill as a normal Linux user; do not disable the Chromium sandbox');
const binary = path.resolve(process.argv[2] ?? '');
const evidence = path.resolve(process.argv[3] ?? '');
const checkout = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
if (!process.argv[2] || !existsSync(binary) || !process.argv[3]
  || evidence === checkout || evidence.startsWith(checkout + path.sep)) throw new Error('Provide the lib-test executable and an external evidence directory');
if (!process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE || !path.isAbsolute(process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE)) throw new Error('Configure an absolute native Chromium executable');
mkdirSync(evidence, { recursive: true });
const scratch = mkdtempSync(path.join(tmpdir(), 'cx-mdint-host-drill-'));
const results = [];
try {
  for (const [name, test, artifacts] of [
    ['kernel-browser-host', 'runtime::router::tests::kernel_browser::kernel_browser_linux_integration_drill', ['MD4-PASS.txt', 'screenshot.png']],
    ['integrated-app-browser', 'runtime::router::tests::user_app_views::user_app_view_kernel_browser_integration_drill', ['MDINT-PASS.txt', 'MDINT-APP.png']],
  ]) {
    const root = path.join(scratch, name);
    for (const part of ['home/chariox', 'tmp', 'logs']) mkdirSync(path.join(root, part), { recursive: true, mode: 0o700 });
    const fd = openSync(path.join(evidence, name + '.log'), 'w');
    let result;
    try {
      result = spawnSync(binary, ['--ignored', '--exact', test, '--nocapture'], {
        cwd: root, stdio: ['ignore', fd, fd], timeout: 180_000,
        env: { ...process.env, HOME: path.join(root, 'home'), TMPDIR: path.join(root, 'tmp'),
          CHARIOX_HOME: path.join(root, 'home/chariox'), CHARIOX_LOG_DIR: path.join(root, 'logs'),
          CHARIOX_MD4_DRILL_ROOT: root, CHARIOX_MDINT_DRILL_ROOT: root,
          CHARIOX_KERNEL_BROWSER_HEADLESS: '1', RUST_TEST_THREADS: '1' },
      });
    } finally { closeSync(fd); }
    for (const artifact of artifacts) if (existsSync(path.join(root, artifact))) copyFileSync(path.join(root, artifact), path.join(evidence, name + '-' + artifact));
    results.push({ name, test, exitCode: result.status, error: result.error?.code ?? null });
    writeFileSync(path.join(evidence, 'results.json'), JSON.stringify({ protocol: 461, relay: 92,
      scope: 'Real production kernel router/controller/sandboxed host Chromium, signed fixture/frontend, fixed ABI worker and synthetic passkey; dev-stub provider MCP admission. No model, live provider/Vault, client/relay projection or Mac claim.', results }, null, 2));
    console.log(`MD-4 ${name}: ${result.status === 0 ? 'PASS' : 'FAIL'}`);
    if (result.status !== 0) process.exitCode = 1;
  }
} finally {
  // Only the disposable directory created here; no shared profile/account state.
  // Signed release payload directories are sealed 0500. Restore owner write
  // permission only under this scratch root so non-root cleanup can unlink them.
  const unseal = directory => {
    if (!lstatSync(directory).isDirectory()) return;
    chmodSync(directory, 0o700);
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      if (entry.isDirectory() && !entry.isSymbolicLink()) unseal(path.join(directory, entry.name));
    }
  };
  unseal(scratch);
  rmSync(scratch, { recursive: true, force: true });
}
