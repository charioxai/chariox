import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, readFileSync } from 'node:fs';
import { mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from 'node:fs/promises';
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
const SUPPORT = 'Library/Application Support/Chariox';
const darwin = { skip: process.platform !== 'darwin' && 'needs macOS packaging tools and BSD stat' };
const sha256 = text => createHash('sha256').update(text).digest('hex');

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
  const inventory = '{"schema":"fixture"}\n';
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
    await assert.rejects(packagePlan(parseArguments(args(copy, ['--unsigned']), {})), pattern);
  };
  await refused(dir => rm(join(dir, 'bin/chariox-kernel')), /missing|no bin\/chariox-kernel/u);
  await refused(dir => writeFile(join(dir, 'bin/helper'), ARM64), /bin\/helper is not a Chariox release binary/u);
  await refused(dir => writeFile(join(dir, 'bin/chariox-relay'), '#!/bin/sh\n'), /not a Mach-O/u);
  await refused(dir => writeFile(join(dir, 'bin/chariox-relay'), X86_64), /chariox-relay is x86_64, but bin\/chariox-kernel is arm64/u);
  await refused(dir => symlink('/etc/hosts', join(dir, 'runtime/sdk/hosts')), /link or special file/u);
  await refused(dir => writeFile(join(dir, 'libexec/extra'), ARM64), /libexec\/ must hold only/u);
  await refused(dir => rm(join(dir, 'runtime/runtime-inventory.sig')), /no runtime-inventory\.sig/u);
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
  assert.deepEqual(unsigned.checks, []);
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
// pkgutil included, below it. The stubs log their arguments.
async function host(t, { installerStatus = 0 } = {}) {
  const root = await scratch(t);
  const R = join(root, 'host');
  const log = join(root, 'calls.log');
  const stub = async (path, body) => {
    await mkdir(dirname(join(R, path)), { recursive: true });
    await writeFile(join(R, path), `#!/bin/bash\nprintf '%s %s\\n' "${path.split('/').pop()}" "$*" >> '${log}'\n${body}\n`, { mode: 0o755 });
  };
  await stub('bin/launchctl', '[ "$1" != print ]');
  await stub('usr/sbin/pkgutil', 'true');
  await stub(`${PACKAGE_DIR}/chariox-app-runtime-install`,
    `[ "$1" != cleanup ] || rm -rf "${R}/${SUPPORT}/AppRuntimes/$3"\nexit ${installerStatus}`);
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
  for (const name of [digest, OLD, 'not-a-generation']) await mkdir(join(fake.R, SUPPORT, 'AppRuntimes', name), { recursive: true });
  const result = await postinstall(fake, digest, 'pinned\n');
  assert.equal(result.status, 0, result.stderr);
  const uid = process.getuid();
  assert.deepEqual(fake.calls(), [
    `chariox-app-runtime-install install --source ${join(fake.R, PACKAGE_DIR, `staging/runtime-${digest}`)} `
      + `--trusted-public-key-hex ${KEY} --inventory-sha256 ${digest}`,
    `launchctl bootout gui/${uid}/dev.chariox.kernel`,
    `launchctl bootstrap gui/${uid} ${join(fake.R, 'Library/LaunchAgents/dev.chariox.kernel.plist')}`,
    `chariox-app-runtime-install cleanup --inventory-sha256 ${OLD}`,
  ]);
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

test('start-kernel sets the kernel environment and applies kernel.env', async t => {
  const root = await scratch(t);
  const home = join(root, 'home');
  await mkdir(join(home, '.config/chariox'), { recursive: true });
  await writeFile(join(home, '.config/chariox/kernel.env'),
    '# comment\nCHARIOX_KERNEL_PORT=4999\nnot a pair\n1BAD=x\nCHARIOX_KERNEL_HOST=0.0.0.0\nCHARIOX_TOKEN_FILE=a=b c');
  await mkdir(join(root, 'usr/local/bin'), { recursive: true });
  await writeFile(join(root, 'usr/local/bin/chariox-kernel'), '#!/bin/sh\npwd\nenv | grep ^CHARIOX_ | sort\n', { mode: 0o755 });
  const script = join(root, 'start-kernel.sh');
  await writeFile(script, await renderTemplate('start-kernel.sh', { ROOT: root }), { mode: 0o755 });
  const result = spawnSync(script, [], { encoding: 'utf8', env: { HOME: home, PATH: '/usr/bin:/bin' } });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stderr.match(/not KEY=VALUE/gu).length, 2);
  assert.deepEqual(readFileSync(join(home, '.chariox/logs/kernel.launchd.log'), 'utf8').trim().split('\n'), [
    home, `CHARIOX_HOME=${home}/.chariox`, 'CHARIOX_KERNEL_HOST=0.0.0.0', 'CHARIOX_KERNEL_PORT=4999',
    `CHARIOX_LOG_DIR=${home}/.chariox/logs`, 'CHARIOX_TOKEN_FILE=a=b c']);
});

test('an unsigned package builds from a release bundle with the pinned postinstall', darwin, async t => {
  const fixture = await bundle(t);
  const { receipt } = await buildMacosPkg(parseArguments(args(fixture, ['--unsigned']), {}));
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

test('a signed build requires Developer ID code and an accepted notarization', darwin, async t => {
  const fixture = await bundle(t);
  const real = command => spawnSync(command[0], command.slice(1), { encoding: 'utf8' });
  const fake = ({ authority = 'Developer ID Application: Example Owner (ABCDE12345)', status = 'Accepted' } = {}) => command => {
    const tool = command[0].split('/').pop();
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
