import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { chmod, mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir, userInfo } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { ADMISSION_CONTRACT, ADMISSION_LABEL, buildMacosPkg, CONTEXT_DIR, formatPlan, packagePlan, PACKAGE_DIR, parseArguments, renderTemplate, UsageError } from './build-macos-pkg.mjs';

const KEY = 'a'.repeat(64);
const OLD = 'b'.repeat(64);
const IDENTITY = 'Developer ID Installer: Example Owner (ABCDE12345)';
const PROFILE = 'chariox-notary';
const ARM64 = Buffer.from([0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00, 0x00, 0x01]);
const X86_64 = Buffer.from([0xcf, 0xfa, 0xed, 0xfe, 0x07, 0x00, 0x00, 0x01]);
// A universal (fat) header with arm64 and x86_64 slices.
const UNIVERSAL = Buffer.from('cafebabe00000002' + '0100000c' + '00'.repeat(16) + '01000007' + '00'.repeat(16), 'hex');
const INSTALLER_TOOL = 'chariox-app-runtime-install';
const SUPPORT = 'Library/Application Support/Chariox';
const CONTEXT = 'share/chariox/slice-build-context';
const PROVISIONER = 'apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh';
// A slice build context in the release bundle's layout: the provisioner, a dot file and plain sources.
const CONTEXT_FILES = { [PROVISIONER]: '#!/bin/bash\n', 'apps/kernel/.charioxignore': 'target\n',
  'apps/kernel/slice-linux-docker/runtime-source-roots.txt': 'Cargo.toml\napps/kernel\n', 'Cargo.toml': '[workspace]\n' };
const CONTEXT_BYTES = Object.values(CONTEXT_FILES).join('').length;
// A release kernel with the installed-context lookup holds its path as a string constant.
const LOOKUP_KERNEL = `chariox-kernel\0${CONTEXT}\0${ADMISSION_CONTRACT}\0`;
const contextManifest = () => Object.entries(CONTEXT_FILES).map(([path, text]) => `${sha256(text)}  /${CONTEXT_DIR}/${path}\n`).join('');
const darwin = { skip: process.platform !== 'darwin' && 'needs macOS packaging tools and BSD stat' };
const sha256 = text => createHash('sha256').update(text).digest('hex');
async function until(condition, ms = 15000) {
  for (const end = Date.now() + ms; !condition();) {
    if (Date.now() > end) throw new Error('timed out');
    await new Promise(done => setTimeout(done, 100));
  }
}

async function scratch(t) {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'macos-pkg-')));
  t.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

// Fake Mach-O headers: enough for the bundle check and pkgbuild; nothing here runs.
async function bundle(t) {
  const root = await scratch(t);
  const dir = join(root, 'bundle');
  await mkdir(join(dir, 'bin'), { recursive: true });
  for (const name of ['chariox-kernel', 'chariox', 'chariox-app-package', 'chariox-app-wasm-worker'])
    await writeFile(join(dir, 'bin', name), Buffer.concat([ARM64, Buffer.from(name === 'chariox-kernel' ? LOOKUP_KERNEL : name)]), { mode: 0o755 });
  for (const [path, text] of Object.entries(CONTEXT_FILES)) {
    await mkdir(dirname(join(dir, CONTEXT, path)), { recursive: true });
    await writeFile(join(dir, CONTEXT, path), text);
    // Group-writable sources still install as root's 0644, and the provisioner as 0755.
    await chmod(join(dir, CONTEXT, path), path === PROVISIONER ? 0o775 : 0o664);
  }
  await writeFile(join(dir, 'SHA256SUMS'), 'not packaged\n');
  return { root, dir, output: join(root, 'Chariox-1.2.3.pkg') };
}

const args = (fixture, extra = []) => ['--bundle', fixture.dir, '--output', fixture.output, '--version', '1.2.3',
  ...extra];
const signed = fixture => parseArguments(args(fixture, ['--identity', IDENTITY, '--keychain-profile', PROFILE]), {});

test('arguments take the installer identity and profile from flags or the environment, or build unsigned', () => {
  const base = ['--bundle', 'b', '--output', 'o.pkg', '--version', '1.0'];
  const fromEnv = parseArguments(base, { CHARIOX_INSTALLER_IDENTITY: IDENTITY, CHARIOX_NOTARY_PROFILE: PROFILE });
  assert.equal(fromEnv.teamId, 'ABCDE12345');
  assert.equal(fromEnv.keychainProfile, PROFILE);
  assert.equal(fromEnv.dryRun, false);
  const unsigned = parseArguments([...base, '--unsigned', '--dry-run'], { CHARIOX_INSTALLER_IDENTITY: IDENTITY });
  assert.deepEqual([unsigned.unsigned, unsigned.dryRun, unsigned.identity, unsigned.teamId], [true, true, null, null]);
  assert.throws(() => parseArguments([...base, '--unsigned', '--identity', IDENTITY], {}), /--unsigned takes no/u);
  for (const identity of ['-', 'Developer ID Application: Example Owner (ABCDE12345)', 'Developer ID Installer: Example Owner', ''])
    assert.throws(() => parseArguments([...base, '--identity', identity, '--keychain-profile', PROFILE], {}), UsageError, identity);
  assert.throws(() => parseArguments([...base, '--identity', IDENTITY], {}), /keychain profile/u);
  assert.throws(() => parseArguments(base.map(word => word === '1.0' ? '1.0-beta' : word), {}), /--version must be/u);
  assert.throws(() => parseArguments(base.map(word => word === 'o.pkg' ? 'o.zip' : word), {}), /\.pkg/u);
  for (const flag of ['--password', '--apple-id', '--team-id'])
    assert.throws(() => parseArguments([...base, flag, 'secret-value'], {}), error => error.message === `unsupported argument ${flag}`);
  assert.throws(() => parseArguments([...base, 'secret-value'], {}), error => !error.message.includes('secret-value'));
  assert.throws(() => parseArguments([...base, '--password secret-value'], {}), error => error.message === 'unsupported argument --password');
  assert.throws(() => parseArguments([...base, '--unsigned', '--unsigned'], {}), /duplicate --unsigned/u);
});

test('the release bundle\'s darwin layout is accepted, and chariox-cli is packaged only when present', async t => {
  // release-bundle.mjs PLATFORMS['darwin-arm64']: bin/chariox, chariox-kernel, chariox-relay, chariox-app-package;
  // libexec/chariox-app-runtime-install; runtime/; share/chariox/slice-build-context; LICENSE and the signed manifest.
  const fixture = await bundle(t);
  await writeFile(join(fixture.dir, 'bin/chariox-relay'), Buffer.concat([ARM64, Buffer.from('relay')]), { mode: 0o755 });
  await rm(join(fixture.dir, 'SHA256SUMS'));
  for (const name of ['LICENSE', 'manifest.json', 'manifest.sig']) await writeFile(join(fixture.dir, name), 'not packaged\n');
  const plan = await packagePlan(parseArguments(args(fixture, ['--unsigned']), {}));
  assert.deepEqual(plan.release.binaries.map(binary => binary.name), ['chariox', 'chariox-app-package', 'chariox-app-wasm-worker', 'chariox-kernel', 'chariox-relay']);
  assert.deepEqual(plan.release.ignored, ['LICENSE', 'manifest.json', 'manifest.sig']);
  assert.equal(plan.payload.find(entry => entry.path === `${PACKAGE_DIR}/bin.sha256`).content.split('\n').filter(Boolean)
    .map(line => line.split('  ')[1]).join(), '/usr/local/bin/chariox,/usr/local/bin/chariox-app-package,/usr/local/bin/chariox-app-wasm-worker,/usr/local/bin/chariox-kernel,/usr/local/bin/chariox-relay');
  // An older bundle's chariox-cli still installs beside it.
  await writeFile(join(fixture.dir, 'bin/chariox-cli'), Buffer.concat([ARM64, Buffer.from('cli')]), { mode: 0o755 });
  const older = await packagePlan(parseArguments(args(fixture, ['--unsigned']), {}));
  assert.ok(older.payload.some(entry => entry.path === 'usr/local/bin/chariox-cli' && entry.mode === 0o755));
  assert.match(older.payload.find(entry => entry.path === `${PACKAGE_DIR}/bin.sha256`).content, /  \/usr\/local\/bin\/chariox-cli\n/u);
});

test('the package puts the slice build context where this tree\'s release kernel looks for it', async () => {
  // installed_slice_script looks in <the parent of its canonical executable's bin/>/RELEASE_SLICE_BUILD_CONTEXT,
  // here /usr/local (realpath /usr/local/bin is itself), before /usr/lib; the kernel's own test covers a link to the executable.
  const kernel = await readFile(join(dirname(fileURLToPath(import.meta.url)), '../apps/kernel/src/slice/local_docker.rs'), 'utf8');
  const constant = name => kernel.match(new RegExp(`const ${name}: &str =\\s*"([^"]+)";`, 'u'))?.[1];
  assert.equal(constant('RELEASE_SLICE_BUILD_CONTEXT'), CONTEXT);
  assert.equal(join(dirname(dirname('/usr/local/bin/chariox-kernel')), constant('RELEASE_SLICE_BUILD_CONTEXT')), `/${CONTEXT_DIR}`);
  assert.equal(constant('SLICE_DOCKER_PROVISIONER'), PROVISIONER);
  assert.match(kernel, /if !cfg!\(debug_assertions\) \{[^}]*return installed_slice_script\(/u);
});

const listen = () => new Promise(done => { const server = createServer().listen(0, '127.0.0.1', () => done(server)); });

test('start-kernel sets the kernel environment and a login shell\'s PATH, applies kernel.env, and waits for a taken endpoint', async t => {
  const root = await scratch(t);
  const home = join(root, 'home');
  const busy = await listen();
  t.after(() => busy.listening && busy.close());
  const probe = await listen();
  const free = probe.address().port;
  await new Promise(done => probe.close(done));
  await mkdir(join(home, '.config/chariox'), { recursive: true });
  const env = port => writeFile(join(home, '.config/chariox/kernel.env'),
    `# comment\nCHARIOX_KERNEL_PORT=${port}\nnot a pair\n1BAD=x\nCHARIOX_KERNEL_HOST=127.0.0.1\nCHARIOX_TOKEN_FILE=a=b c`);
  await env(free);
  // The kernel runs docker and the provider CLIs from directories outside launchd's PATH:
  // Homebrew's, /etc/paths and /etc/paths.d (path_helper), and the user's own.
  const tool = async (path, name) => {
    await mkdir(path, { recursive: true });
    await writeFile(join(path, name), '#!/bin/sh\n', { mode: 0o755 });
  };
  await tool(join(root, 'opt/homebrew/bin'), 'colima');
  await tool(join(root, 'usr/local/bin'), 'docker');
  await tool(join(root, 'Applications/Tool/bin'), 'paths-d-tool');
  await tool(join(home, '.local/bin'), 'claude');
  await tool(join(home, '.docker/bin'), 'docker-credential-desktop');
  // path_helper puts /etc/paths and /etc/paths.d first and keeps the inherited entries.
  await mkdir(join(root, 'usr/libexec'));
  await writeFile(join(root, 'usr/libexec/path_helper'),
    `#!/bin/sh\nprintf 'PATH="%s"; export PATH;\\n' "${root}/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${root}/Applications/Tool/bin"\n`, { mode: 0o755 });
  await writeFile(join(root, 'usr/local/bin/chariox-kernel'), '#!/bin/sh\npwd\nenv | grep ^CHARIOX_ | sort\necho "PATH=$PATH"\n'
    + 'for name in docker colima claude docker-credential-desktop paths-d-tool; do command -v "$name"; done\ntrue\n', { mode: 0o755 });
  const script = join(root, 'start-kernel.sh');
  await writeFile(script, await renderTemplate('start-kernel.sh', { ROOT: root }), { mode: 0o755 });
  const launchd = { HOME: home, PATH: '/usr/bin:/bin:/usr/sbin:/sbin' };
  const result = spawnSync(script, [], { encoding: 'utf8', env: launchd });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stderr.match(/not KEY=VALUE/gu).length, 2);
  const log = join(home, '.chariox/logs/kernel.launchd.log');
  assert.deepEqual(readFileSync(log, 'utf8').trim().split('\n'), [
    home, `CHARIOX_HOME=${home}/.chariox`, 'CHARIOX_KERNEL_HOST=127.0.0.1', `CHARIOX_KERNEL_PORT=${free}`,
    `CHARIOX_LOG_DIR=${home}/.chariox/logs`, 'CHARIOX_TOKEN_FILE=a=b c',
    `PATH=${root}/opt/homebrew/bin:${root}/opt/homebrew/sbin:${root}/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:`
      + `${root}/Applications/Tool/bin:${home}/.local/bin:${home}/.docker/bin`,
    `${root}/usr/local/bin/docker`, `${root}/opt/homebrew/bin/colima`, `${home}/.local/bin/claude`,
    `${home}/.docker/bin/docker-credential-desktop`, `${root}/Applications/Tool/bin/paths-d-tool`]);
  // A PATH in kernel.env replaces it.
  await rm(log);
  await writeFile(join(home, '.config/chariox/kernel.env'), `CHARIOX_KERNEL_PORT=${free}\nPATH=/usr/bin:/bin:${home}/.local/bin\n`);
  assert.equal(spawnSync(script, [], { encoding: 'utf8', env: launchd }).status, 0);
  const overridden = readFileSync(log, 'utf8');
  assert.match(overridden, new RegExp(`^PATH=/usr/bin:/bin:${home}/\\.local/bin\n`, 'mu'));
  assert.match(overridden, new RegExp(`^${home}/\\.local/bin/claude$`, 'mu'));
  // None of the default directories remain (a host may still have its own /usr/bin/docker).
  assert.doesNotMatch(overridden, new RegExp(`^${root}/(?:usr/local|opt/homebrew|Applications)/|\\.docker/bin`, 'mu'));
  // Occupied, then released: the kernel waits instead of failing, and starts once the endpoint is free.
  await rm(log);
  const port = busy.address().port;
  await env(port);
  const waiting = spawn(script, [], { env: { HOME: home, PATH: '/usr/bin:/bin' }, stdio: 'ignore' });
  const exited = new Promise(done => waiting.on('exit', done));
  await until(() => existsSync(log) && readFileSync(log, 'utf8').includes(`127.0.0.1:${port} is in use; waiting for it to be free`));
  assert.doesNotMatch(readFileSync(log, 'utf8'), /CHARIOX_HOME=/u);
  await new Promise(done => busy.close(done));
  assert.equal(await exited, 0);
  assert.match(readFileSync(log, 'utf8'), new RegExp(`127\\.0\\.0\\.1:${port} is free; starting the kernel\n[^]*CHARIOX_HOME=${home}/\\.chariox`, 'u'));
});

test('MP-11 F20 signed and unsigned plans only invoke system packaging tools', async t => {
  const fixture = await bundle(t);
  for (const options of [signed(fixture), parseArguments(args(fixture, ['--unsigned']), {})]) {
    const plan = await packagePlan(options);
    assert.equal([...plan.checks, ...plan.steps].some(step => step.command[0].startsWith(fixture.dir + '/')), false);
  }
});

// MP-08 / MP-09 / MP-10 / MP-11: release validation never executes bundle code.
test('a WASM-only package validates architectures and signs the worker with its kernel', async t => {
  const fixture = await bundle(t)
  const plan = await packagePlan(signed(fixture))
  const paths = plan.payload.map(entry => entry.path)
  assert.ok(paths.includes('usr/local/bin/chariox-app-wasm-worker'))
  assert.ok(!paths.some(path => /AppRuntimes|runtime-inventory|app-runtime-install/.test(path)))
  assert.match(plan.postinstall, /Docker admission locks/)
  assert.doesNotMatch(plan.postinstall, /runtime-enrollment|runtime-inventory|app-runtime-install/)
  assert.ok(plan.checks.some(check => check.file.path.endsWith('/chariox-app-wasm-worker')))
  assert.ok(plan.checks.every(check => check.command[0] === '/usr/bin/codesign'))
  const unsigned = await packagePlan(parseArguments(args(fixture, ['--unsigned']), {}))
  assert.deepEqual(unsigned.checks, [])
  assert.match(formatPlan(unsigned), /postinstall installs the WASM worker with the kernel/)
})

test('package preparation refuses a missing, linked or foreign-architecture WASM worker', async t => {
  for (const fault of ['missing', 'link', 'architecture']) {
    const fixture = await bundle(t)
    const worker = join(fixture.dir, 'bin/chariox-app-wasm-worker')
    await rm(worker)
    if (fault === 'link') await symlink(join(fixture.dir, 'bin/chariox-kernel'), worker)
    if (fault === 'architecture') await writeFile(worker, X86_64, {mode: 0o755})
    await assert.rejects(packagePlan(parseArguments(args(fixture, ['--unsigned']), {})),
      fault === 'missing' ? /no bin\/chariox-app-wasm-worker/ : fault === 'link' ? /regular file|link/ : /is x86_64/)
  }
})

test('retired runtime installer flags are refused at the package boundary', () => {
  assert.throws(() => parseArguments(['--bundle', 'b', '--output', 'o.pkg', '--version', '1.0', '--runtime-key', KEY]), /unsupported argument --runtime-key/)
})
