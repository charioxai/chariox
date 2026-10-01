import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, readdirSync, readFileSync } from 'node:fs';
import { mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir, userInfo } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { buildMacosPkg, formatPlan, packagePlan, PACKAGE_DIR, parseArguments, renderTemplate, UsageError } from './build-macos-pkg.mjs';

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
  await mkdir(join(dir, 'libexec'));
  await mkdir(join(dir, 'runtime/sdk'), { recursive: true });
  for (const name of ['chariox-kernel', 'chariox-cli', 'chariox-app-package'])
    await writeFile(join(dir, 'bin', name), Buffer.concat([ARM64, Buffer.from(name)]), { mode: 0o755 });
  await writeFile(join(dir, 'libexec/chariox-app-runtime-install'), Buffer.concat([ARM64, Buffer.from('installer')]), { mode: 0o755 });
  const inventory = '{"schema":"fixture","target":"darwin-arm64"}\n';
  await writeFile(join(dir, 'runtime/runtime-inventory.json'), inventory, { mode: 0o444 });
  await writeFile(join(dir, 'runtime/runtime-inventory.sig'), 'signature', { mode: 0o444 });
  await writeFile(join(dir, 'runtime/chariox-app-worker'), Buffer.concat([ARM64, Buffer.from('worker')]), { mode: 0o555 });
  await writeFile(join(dir, 'runtime/sdk/index.js'), 'export {};\n', { mode: 0o444 });
  await writeFile(join(dir, 'SHA256SUMS'), 'not packaged\n');
  return { root, dir, digest: sha256(inventory), output: join(root, 'Chariox-1.2.3.pkg') };
}

const args = (fixture, extra = []) => ['--bundle', fixture.dir, '--output', fixture.output, '--version', '1.2.3',
  '--runtime-key', KEY, '--runtime-digest', fixture.digest, ...extra];
const signed = fixture => parseArguments(args(fixture, ['--identity', IDENTITY, '--keychain-profile', PROFILE]), {});

test('arguments take the installer identity and profile from flags or the environment, or build unsigned', () => {
  const base = ['--bundle', 'b', '--output', 'o.pkg', '--version', '1.0', '--runtime-key', KEY, '--runtime-digest', OLD];
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
  assert.throws(() => parseArguments(base.map(word => word === KEY ? 'A'.repeat(64) : word), {}), /--runtime-key must be/u);
  assert.throws(() => parseArguments(base.map(word => word === '1.0' ? '1.0-beta' : word), {}), /--version must be/u);
  assert.throws(() => parseArguments(base.map(word => word === 'o.pkg' ? 'o.zip' : word), {}), /\.pkg/u);
  for (const flag of ['--password', '--apple-id', '--team-id'])
    assert.throws(() => parseArguments([...base, flag, 'secret-value'], {}), error => error.message === `unsupported argument ${flag}`);
  assert.throws(() => parseArguments([...base, 'secret-value'], {}), error => !error.message.includes('secret-value'));
  assert.throws(() => parseArguments([...base, '--password secret-value'], {}), error => error.message === 'unsupported argument --password');
  assert.throws(() => parseArguments([...base, '--unsigned', '--unsigned'], {}), /duplicate --unsigned/u);
});

test('the bundle check refuses a different runtime, missing or stray binaries, links and mixed architectures', async t => {
  const fixture = await bundle(t);
  const plan = await packagePlan(parseArguments(args(fixture, ['--unsigned']), {}));
  assert.deepEqual(plan.release.ignored, ['SHA256SUMS']);
  assert.deepEqual(plan.release.archs, ['arm64']);
  await assert.rejects(packagePlan(parseArguments(args({ ...fixture, digest: OLD }, ['--unsigned']), {})), /not the --runtime-digest/u);
  const refused = async (change, pattern) => {
    const copy = await bundle(t);
    await change(copy.dir);
    const digest = sha256(readFileSync(join(copy.dir, 'runtime/runtime-inventory.json')));
    await assert.rejects(packagePlan(parseArguments(args({ ...copy, digest }, ['--unsigned']), {})), pattern);
  };
  await refused(dir => rm(join(dir, 'bin/chariox-kernel')), /missing|no bin\/chariox-kernel/u);
  await refused(dir => writeFile(join(dir, 'bin/helper'), ARM64), /bin\/helper is not a Chariox release binary/u);
  await refused(dir => writeFile(join(dir, 'bin/chariox-relay'), '#!/bin/sh\n'), /not a Mach-O/u);
  await refused(dir => writeFile(join(dir, 'bin/chariox-relay'), X86_64), /chariox-relay is x86_64, but bin\/chariox-kernel is arm64/u);
  await refused(dir => symlink('/etc/hosts', join(dir, 'runtime/sdk/hosts')), /link or special file/u);
  await refused(dir => writeFile(join(dir, 'libexec/extra'), ARM64), /libexec\/ must hold only/u);
  await refused(dir => rm(join(dir, 'runtime/runtime-inventory.sig')), /no runtime-inventory\.sig/u);
  // The runtime files are read-only, as in a release.
  const replace = (path, bytes) => rm(path).then(() => writeFile(path, bytes));
  // The runtime must match the package's one architecture, by its inventory target and every Mach-O.
  await refused(dir => replace(join(dir, 'runtime/chariox-app-worker'), X86_64), /runtime\/chariox-app-worker is x86_64, but bin\/chariox-kernel is arm64/u);
  await refused(dir => replace(join(dir, 'runtime/runtime-inventory.json'), '{"target":"darwin-x64"}'), /runtime\/ targets darwin-x64, but bin\/chariox-kernel is arm64/u);
  await refused(dir => replace(join(dir, 'runtime/runtime-inventory.json'), 'not json'), /not JSON/u);
  await refused(async dir => {
    for (const path of ['bin/chariox-kernel', 'bin/chariox-cli', 'bin/chariox-app-package', `libexec/${INSTALLER_TOOL}`]) await writeFile(join(dir, path), UNIVERSAL);
  }, /arm64\+x86_64; build one package per architecture/u);
  await writeFile(fixture.output, '');
  await assert.rejects(packagePlan(parseArguments(args(fixture, ['--unsigned']), {})), /already exists/u);
});

test('the dry-run plan pins the runtime and orders the release signing steps', async t => {
  const fixture = await bundle(t);
  const plan = await packagePlan(signed(fixture));
  const text = formatPlan(plan);
  assert.match(text, /^# Dry run: nothing below was executed or written\./u);
  assert.match(text, new RegExp(`postinstall pins runtime key ${KEY} and inventory ${fixture.digest}`, 'u'));
  assert.match(text, /# payload 0755 \/usr\/local\/bin\/chariox-kernel/u);
  assert.match(text, /# payload 0644 \/Library\/LaunchAgents\/dev\.chariox\.kernel\.plist/u);
  assert.match(text, /# payload 0555 \/usr\/local\/libexec\/chariox\/chariox-app-runtime-install/u);
  // Every Mach-O, the runtime worker included, must be Developer ID code of the team.
  assert.equal(plan.checks.filter(step => step.phase === 'code-describe').length, 5);
  assert.deepEqual(plan.checks[0].command, [join(fixture.dir, 'libexec', INSTALLER_TOOL)]);
  assert.deepEqual(plan.steps.map(step => step.phase), ['component', 'payload', 'product', 'expand', 'sign', 'signature',
    'notarize', 'notary-log', 'staple', 'staple-check', 'gatekeeper']);
  const command = phase => plan.steps.find(step => step.phase === phase).command.join(' ');
  assert.match(command('component'), /--identifier dev\.chariox\.pkg --version 1\.2\.3 .*--min-os-version 13\.5/u);
  assert.equal(command('sign'), `/usr/bin/productsign --sign ${IDENTITY} --timestamp ${plan.paths.product} ${plan.output}`);
  assert.match(command('notarize'), new RegExp(`notarytool submit ${plan.output} --keychain-profile ${PROFILE} --wait`, 'u'));
  assert.equal(command('staple'), `/usr/bin/xcrun stapler staple ${plan.output}`);
  assert.match(plan.distribution, /hostArchitectures="arm64"/u);
  assert.match(plan.distribution, /<os-version min="13\.5"\/>/u);
  assert.match(plan.distribution, /enable_localSystem="true"/u);
  const unsigned = await packagePlan(parseArguments(args(fixture, ['--unsigned']), {}));
  assert.deepEqual(unsigned.checks.map(step => step.phase), ['installer-probe']);
  assert.deepEqual(unsigned.steps.map(step => step.phase), ['component', 'payload', 'product', 'expand']);
  assert.equal(unsigned.paths.product, unsigned.output);
  assert.match(formatPlan(unsigned), /# unsigned: a test package/u);
});

test('the package scripts render the pinned runtime with an empty root', async () => {
  const values = { ROOT: '', VERSION: '1.2.3', RUNTIME_KEY: KEY, RUNTIME_DIGEST: OLD };
  const postinstall = await renderTemplate('postinstall', values);
  assert.match(postinstall, /^R=''$/mu);
  assert.match(postinstall, new RegExp(`^RUNTIME_KEY='${KEY}'$`, 'mu'));
  assert.match(postinstall, new RegExp(`^RUNTIME_DIGEST='${OLD}'$`, 'mu'));
  for (const name of ['postinstall', 'uninstall.sh', 'start-kernel.sh']) {
    const text = await renderTemplate(name, values);
    assert.doesNotMatch(text, /@@/u);
    assert.equal(spawnSync('/bin/bash', ['-n'], { input: text }).status, 0, name);
  }
  await assert.rejects(renderTemplate('postinstall', { ROOT: '' }), /needs VERSION/u);
  await assert.rejects(renderTemplate('uninstall.sh', { ROOT: "x'; rm -rf /" }), /quotes/u);
});

// A fake host root: the rendered scripts resolve every path, launchctl and
// pkgutil included, below it. The stubs log their arguments. The installer stub
// keeps the real limits: install refuses a ninth generation, and cleanup refuses
// a generation whose lease is held (.busy, or .held until bootout restarts the
// kernel that holds it).
async function host(t, { installerStatus = 0 } = {}) {
  const root = await scratch(t);
  const R = join(root, 'host');
  const log = join(root, 'calls.log');
  const stub = async (path, body) => {
    await mkdir(dirname(join(R, path)), { recursive: true });
    await writeFile(join(R, path), `#!/bin/bash\nprintf '%s %s\\n' "${path.split('/').pop()}" "$*" >> '${log}'\n${body}\n`, { mode: 0o755 });
  };
  const runtimes = join(R, SUPPORT, 'AppRuntimes');
  await stub('bin/launchctl', `[ "$1" != bootout ] || rm -f '${runtimes}'/*/.held\n[ "$1" != print ]`);
  await stub('usr/sbin/pkgutil', 'true');
  await stub(`${PACKAGE_DIR}/chariox-app-runtime-install`, `runtimes='${runtimes}'
case "$1" in
  cleanup) [ ! -e "$runtimes/$3/.busy" ] && [ ! -e "$runtimes/$3/.held" ] || exit 1; rm -rf "$runtimes/$3" ;;
  install)
    [ ${installerStatus} = 0 ] || exit ${installerStatus}
    if [ ! -d "$runtimes/$7" ] && [ "$(ls "$runtimes" 2>/dev/null | grep -cE '^[0-9a-f]{64}$')" -ge 8 ]; then
      echo app_runtime_installer_limit >&2; exit 1
    fi
    mkdir -p "$runtimes/$7" ;;
esac`);
  return { root, R, log, calls: () => existsSync(log) ? readFileSync(log, 'utf8').trim().split('\n') : [] };
}

async function postinstall(fake, digest, inventory) {
  const staged = join(fake.R, PACKAGE_DIR, `staging/runtime-${digest}`);
  await mkdir(staged, { recursive: true });
  await writeFile(join(staged, 'runtime-inventory.json'), inventory);
  const script = join(fake.root, 'postinstall');
  await writeFile(script, await renderTemplate('postinstall', { ROOT: fake.R, VERSION: '1.2.3', RUNTIME_KEY: KEY, RUNTIME_DIGEST: digest }), { mode: 0o755 });
  return spawnSync(script, [join(fake.root, 'Chariox.pkg'), '/', '/', '/'], { encoding: 'utf8' });
}

test('postinstall refuses a staged runtime that is not the pinned one', darwin, async t => {
  const fake = await host(t);
  await mkdir(join(fake.R, 'dev'));
  await writeFile(join(fake.R, 'dev/console'), '');
  const result = await postinstall(fake, sha256('pinned\n'), 'tampered\n');
  assert.equal(result.status, 1);
  assert.match(result.stderr, /not the pinned .*nothing was enrolled/u);
  assert.deepEqual(fake.calls(), []);
  assert.equal(existsSync(join(fake.R, PACKAGE_DIR, 'staging')), false);
});

test('postinstall enrolls the pinned runtime, restarts the console user\'s kernel and retires old generations', darwin, async t => {
  const fake = await host(t);
  const digest = sha256('pinned\n');
  await mkdir(join(fake.R, 'dev'));
  await writeFile(join(fake.R, 'dev/console'), '');
  // OLD is held by the running kernel until its restart; FREE by nothing.
  const FREE = 'c'.repeat(64);
  for (const name of [digest, OLD, FREE, 'not-a-generation']) await mkdir(join(fake.R, SUPPORT, 'AppRuntimes', name), { recursive: true });
  await writeFile(join(fake.R, SUPPORT, 'AppRuntimes', OLD, '.held'), '');
  const result = await postinstall(fake, digest, 'pinned\n');
  assert.equal(result.status, 0, result.stderr);
  const uid = process.getuid();
  assert.deepEqual(fake.calls(), [
    `chariox-app-runtime-install cleanup --inventory-sha256 ${OLD}`,
    `chariox-app-runtime-install cleanup --inventory-sha256 ${FREE}`,
    `chariox-app-runtime-install install --source ${join(fake.R, PACKAGE_DIR, `staging/runtime-${digest}`)} `
      + `--trusted-public-key-hex ${KEY} --inventory-sha256 ${digest}`,
    `launchctl bootout gui/${uid}/dev.chariox.kernel`,
    `launchctl bootstrap gui/${uid} ${join(fake.R, 'Library/LaunchAgents/dev.chariox.kernel.plist')}`,
    `chariox-app-runtime-install cleanup --inventory-sha256 ${OLD}`,
  ]);
  assert.match(result.stdout, new RegExp(`kept App runtime ${OLD}: it is enrolled or still in use[^]*retired App runtime ${FREE}[^]*retired App runtime ${OLD}`, 'u'));
  assert.match(result.stdout, new RegExp(`started the kernel for ${userInfo().username}`, 'u'));
  assert.equal(existsSync(join(fake.R, PACKAGE_DIR, 'staging')), false);
});

test('postinstall stops when the installer refuses and skips the agent without a console user', darwin, async t => {
  const refused = await host(t, { installerStatus: 1 });
  await mkdir(join(refused.R, 'dev'));
  await writeFile(join(refused.R, 'dev/console'), '');
  const digest = sha256('pinned\n');
  const failure = await postinstall(refused, digest, 'pinned\n');
  assert.equal(failure.status, 1);
  assert.match(failure.stderr, /refused the runtime; the kernel was not restarted/u);
  assert.deepEqual(refused.calls().map(call => call.split(' ').slice(0, 2).join(' ')), ['chariox-app-runtime-install install']);
  const nobody = await host(t);
  const success = await postinstall(nobody, digest, 'pinned\n');
  assert.equal(success.status, 0, success.stderr);
  assert.match(success.stdout, /no user is logged in at the console/u);
  assert.equal(nobody.calls().some(call => call.startsWith('launchctl')), false);
});

test('postinstall frees unused generations before enrolling at the eight-generation limit', darwin, async t => {
  const fake = await host(t);
  const digest = sha256('pinned\n');
  // Six generations stay in use; two were released since the last install.
  for (let index = 0; index < 8; index += 1) {
    const generation = join(fake.R, SUPPORT, 'AppRuntimes', String(index).repeat(64));
    await mkdir(generation, { recursive: true });
    if (index < 6) await writeFile(join(generation, '.busy'), '');
  }
  const result = await postinstall(fake, digest, 'pinned\n');
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /retired App runtime 6{64}[^]*retired App runtime 7{64}/u);
  const left = readdirSync(join(fake.R, SUPPORT, 'AppRuntimes')).sort();
  assert.deepEqual(left, [...[0, 1, 2, 3, 4, 5].map(index => String(index).repeat(64)), digest].sort());
});

test('uninstall removes what the package installed and keeps changed binaries', darwin, async t => {
  const fake = await host(t);
  const file = async (path, text = '') => {
    await mkdir(dirname(join(fake.R, path)), { recursive: true });
    await writeFile(join(fake.R, path), text);
  };
  await file('Library/LaunchAgents/dev.chariox.kernel.plist');
  await file(`${SUPPORT}/AppRuntime/runtime-enrollment.json`, '{}');
  await file(`${SUPPORT}/AppRuntime/.runtime-installer.lock`);
  await file(`${SUPPORT}/AppRuntimes/${OLD}/.runtime-lease`);
  await file('usr/local/bin/chariox-kernel', 'kernel');
  await file('usr/local/bin/chariox-cli', 'replaced by the user');
  await file(`${PACKAGE_DIR}/start-kernel.sh`);
  await file(`${PACKAGE_DIR}/bin.sha256`, `${sha256('kernel')}  /usr/local/bin/chariox-kernel\n${sha256('cli')}  /usr/local/bin/chariox-cli\n`);
  const script = join(fake.R, PACKAGE_DIR, 'uninstall.sh');
  await writeFile(script, await renderTemplate('uninstall.sh', { ROOT: fake.R }), { mode: 0o755 });
  assert.equal(spawnSync(script, ['--force'], { encoding: 'utf8' }).status, 2);
  const dry = spawnSync(script, ['--dry-run'], { encoding: 'utf8' });
  assert.equal(dry.status, 0, dry.stderr);
  assert.match(dry.stdout, /would withdraw the App runtime enrollment/u);
  assert.match(dry.stdout, new RegExp(`would retire App runtime ${OLD}`, 'u'));
  assert.equal(existsSync(join(fake.R, `${SUPPORT}/AppRuntime/runtime-enrollment.json`)), true);
  assert.equal(fake.calls().some(call => !call.startsWith('launchctl print') && !call.startsWith('pkgutil --pkg-info')), false);
  const result = spawnSync(script, [], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /kept \/usr\/local\/bin\/chariox-cli: it changed/u);
  for (const gone of ['Library/LaunchAgents/dev.chariox.kernel.plist', SUPPORT, 'usr/local/bin/chariox-kernel', PACKAGE_DIR])
    assert.equal(existsSync(join(fake.R, gone)), false, gone);
  assert.equal(readFileSync(join(fake.R, 'usr/local/bin/chariox-cli'), 'utf8'), 'replaced by the user');
  assert.ok(fake.calls().includes(`chariox-app-runtime-install cleanup --inventory-sha256 ${OLD}`));
  assert.ok(fake.calls().includes('pkgutil --forget dev.chariox.pkg'));
});

const listen = () => new Promise(done => { const server = createServer().listen(0, '127.0.0.1', () => done(server)); });

test('start-kernel sets the kernel environment, applies kernel.env, and waits for a taken endpoint', async t => {
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
  await mkdir(join(root, 'usr/local/bin'), { recursive: true });
  await writeFile(join(root, 'usr/local/bin/chariox-kernel'), '#!/bin/sh\npwd\nenv | grep ^CHARIOX_ | sort\n', { mode: 0o755 });
  const script = join(root, 'start-kernel.sh');
  await writeFile(script, await renderTemplate('start-kernel.sh', { ROOT: root }), { mode: 0o755 });
  const result = spawnSync(script, [], { encoding: 'utf8', env: { HOME: home, PATH: '/usr/bin:/bin' } });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stderr.match(/not KEY=VALUE/gu).length, 2);
  const log = join(home, '.chariox/logs/kernel.launchd.log');
  assert.deepEqual(readFileSync(log, 'utf8').trim().split('\n'), [
    home, `CHARIOX_HOME=${home}/.chariox`, 'CHARIOX_KERNEL_HOST=127.0.0.1', `CHARIOX_KERNEL_PORT=${free}`,
    `CHARIOX_LOG_DIR=${home}/.chariox/logs`, 'CHARIOX_TOKEN_FILE=a=b c']);
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

test('an unsigned package builds from a release bundle with the pinned postinstall', darwin, async t => {
  const fixture = await bundle(t);
  // The fixture installer is no real executable; the probe gets the macOS installer's answer to an ordinary user.
  const run = command => command[0].endsWith(INSTALLER_TOOL) ? { status: 1, stdout: '', stderr: 'app_runtime_installer_requires_root\n' }
    : spawnSync(command[0], command.slice(1), { encoding: 'utf8' });
  const { receipt } = await buildMacosPkg(parseArguments(args(fixture, ['--unsigned']), {}), { run });
  assert.equal(receipt.signing, 'unsigned test package: Gatekeeper refuses it');
  assert.equal(receipt.sha256, sha256(readFileSync(fixture.output)));
  assert.deepEqual(receipt.payload.map(entry => entry.path), ['/usr/local/bin/chariox-app-package', '/usr/local/bin/chariox-cli',
    '/usr/local/bin/chariox-kernel', `/${PACKAGE_DIR}/chariox-app-runtime-install`, `/${PACKAGE_DIR}/start-kernel.sh`,
    `/${PACKAGE_DIR}/uninstall.sh`, `/${PACKAGE_DIR}/bin.sha256`, '/Library/LaunchAgents/dev.chariox.kernel.plist']);
  assert.equal(existsSync(`${fixture.output}.build`), false);
  const expanded = join(fixture.root, 'expanded');
  assert.equal(spawnSync('/usr/sbin/pkgutil', ['--expand', fixture.output, expanded]).status, 0);
  assert.match(await readFile(join(expanded, 'chariox.pkg/Scripts/postinstall'), 'utf8'), new RegExp(`RUNTIME_DIGEST='${fixture.digest}'`, 'u'));
});

test('a signed build requires a macOS-capable installer, Developer ID code and an accepted notarization', darwin, async t => {
  const fixture = await bundle(t);
  const real = command => spawnSync(command[0], command.slice(1), { encoding: 'utf8' });
  const fake = ({ authority = 'Developer ID Application: Example Owner (ABCDE12345)', status = 'Accepted',
    installer = 'app_runtime_installer_requires_root' } = {}) => command => {
    const tool = command[0].split('/').pop();
    if (tool === INSTALLER_TOOL) return { status: 1, stdout: '', stderr: `${installer}\n` };
    if (tool === 'codesign') return { status: 0, stdout: '', stderr: command[1] === '--verify' ? 'valid on disk\n'
      : `Authority=${authority}\nTimestamp=Oct 1, 2026\nTeamIdentifier=ABCDE12345\nCodeDirectory v=20500 size=1 flags=0x10000(runtime)\n` };
    if (tool === 'productsign') { copyFileSync(command.at(-2), command.at(-1)); return { status: 0, stdout: '', stderr: '' }; }
    if (tool === 'xcrun' && command[2] === 'submit') return { status: 0, stdout: JSON.stringify({ id: 'abc-123', status }), stderr: '' };
    if (tool === 'xcrun') return { status: 0, stdout: '', stderr: '' };
    if (tool === 'spctl') return { status: 0, stdout: '', stderr: `${command.at(-1)}: accepted\nsource=Notarized Developer ID\n` };
    if (command[1] === '--check-signature') return { status: 0, stderr: '', stdout: 'Status: signed by a developer certificate issued by Apple for distribution\n'
      + 'Signed with a trusted timestamp on: 2026-10-01\n1. Developer ID Installer: Example Owner (ABCDE12345)\n' };
    return real(command);
  };
  // An installer built without macOS enrollment is refused before anything is written.
  await assert.rejects(buildMacosPkg(signed(fixture), { run: fake({ installer: 'app_runtime_installer_platform_unsupported' }) }),
    /cannot enroll on macOS \(it answered "app_runtime_installer_platform_unsupported"\)/u);
  assert.equal(existsSync(`${fixture.output}.build`), false);
  await assert.rejects(buildMacosPkg(signed(fixture), { run: fake({ authority: 'Apple Development: Someone (ZZZZZ99999)' }) }),
    /not Developer ID Application-signed by team ABCDE12345/u);
  assert.equal(existsSync(`${fixture.output}.build`), false);
  await assert.rejects(buildMacosPkg(signed(fixture), { run: fake({ status: 'Invalid' }) }), /notarization Invalid/u);
  for (const path of [fixture.output, `${fixture.output}.build`, `${fixture.output}.notarization-log.json`]) assert.equal(existsSync(path), false);
  const { receipt } = await buildMacosPkg(signed(fixture), { run: fake() });
  assert.deepEqual(receipt.signing.notarization, { submissionId: 'abc-123', status: 'Accepted', log: `${fixture.output}.notarization-log.json` });
  assert.equal(receipt.signing.teamId, 'ABCDE12345');
  assert.equal(existsSync(fixture.output), true);
});
