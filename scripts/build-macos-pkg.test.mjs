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
import { buildMacosPkg, CONTEXT_DIR, formatPlan, packagePlan, PACKAGE_DIR, parseArguments, renderTemplate, UsageError } from './build-macos-pkg.mjs';

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
const LOOKUP_KERNEL = `chariox-kernel\0${CONTEXT}\0`;
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
  await mkdir(join(dir, 'libexec'));
  await mkdir(join(dir, 'runtime/sdk'), { recursive: true });
  for (const name of ['chariox-kernel', 'chariox', 'chariox-app-package'])
    await writeFile(join(dir, 'bin', name), Buffer.concat([ARM64, Buffer.from(name === 'chariox-kernel' ? LOOKUP_KERNEL : name)]), { mode: 0o755 });
  await writeFile(join(dir, 'libexec/chariox-app-runtime-install'), Buffer.concat([ARM64, Buffer.from('installer')]), { mode: 0o755 });
  const inventory = '{"schema":"fixture","target":"darwin-arm64"}\n';
  await writeFile(join(dir, 'runtime/runtime-inventory.json'), inventory, { mode: 0o444 });
  await writeFile(join(dir, 'runtime/runtime-inventory.sig'), 'signature', { mode: 0o444 });
  await writeFile(join(dir, 'runtime/chariox-app-worker'), Buffer.concat([ARM64, Buffer.from('worker')]), { mode: 0o555 });
  await writeFile(join(dir, 'runtime/sdk/index.js'), 'export {};\n', { mode: 0o444 });
  for (const [path, text] of Object.entries(CONTEXT_FILES)) {
    await mkdir(dirname(join(dir, CONTEXT, path)), { recursive: true });
    await writeFile(join(dir, CONTEXT, path), text);
    // Group-writable sources still install as root's 0644, and the provisioner as 0755.
    await chmod(join(dir, CONTEXT, path), path === PROVISIONER ? 0o775 : 0o664);
  }
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
  await refused(dir => rm(join(dir, 'bin/chariox')), /the bundle has no bin\/chariox$/u);
  await refused(dir => writeFile(join(dir, 'bin/helper'), ARM64), /bin\/helper is not a Chariox release binary/u);
  await refused(dir => writeFile(join(dir, 'bin/chariox-relay'), '#!/bin/sh\n'), /not a Mach-O/u);
  await refused(dir => writeFile(join(dir, 'bin/chariox-relay'), X86_64), /chariox-relay is x86_64, but bin\/chariox-kernel is arm64/u);
  await refused(dir => symlink('/etc/hosts', join(dir, 'runtime/sdk/hosts')), /link or special file/u);
  await refused(dir => writeFile(join(dir, 'libexec/extra'), ARM64), /libexec\/ must hold only/u);
  await refused(dir => rm(join(dir, 'runtime/runtime-inventory.sig')), /no runtime-inventory\.sig/u);
  // A release kernel runs local Docker slices only from the bundle's slice build context.
  const noContext = /the bundle has no executable share\/chariox\/slice-build-context\/apps\/kernel\/slice-linux-docker\/provision-linux-docker-slice\.sh/u;
  await refused(dir => rm(join(dir, 'share'), { recursive: true }), noContext);
  await refused(dir => writeFile(join(dir, 'bin/chariox-kernel'), Buffer.concat([ARM64, Buffer.from('an older or debug kernel')])),
    /bin\/chariox-kernel does not look for share\/chariox\/slice-build-context beside its bin\//u);
  await refused(dir => rm(join(dir, CONTEXT, PROVISIONER)), noContext);
  await refused(dir => chmod(join(dir, CONTEXT, PROVISIONER), 0o644), noContext);
  await refused(dir => mkdir(join(dir, 'share/man')), /share\/ must hold only chariox\/slice-build-context/u);
  await refused(dir => symlink('/etc/hosts', join(dir, CONTEXT, 'hosts')), /slice-build-context\/hosts is a link or special file/u);
  for (const name of ['a b.rs', "it's.rs", 'line\nbreak.rs', '..hidden'])
    await refused(dir => writeFile(join(dir, CONTEXT, 'apps', name), ''), /a path the package cannot list/u);
  // The runtime files are read-only, as in a release.
  const replace = (path, bytes) => rm(path).then(() => writeFile(path, bytes));
  // The runtime must match the package's one architecture, by its inventory target and every Mach-O.
  await refused(dir => replace(join(dir, 'runtime/chariox-app-worker'), X86_64), /runtime\/chariox-app-worker is x86_64, but bin\/chariox-kernel is arm64/u);
  await refused(dir => replace(join(dir, 'runtime/runtime-inventory.json'), '{"target":"darwin-x64"}'), /runtime\/ targets darwin-x64, but bin\/chariox-kernel is arm64/u);
  await refused(dir => replace(join(dir, 'runtime/runtime-inventory.json'), 'not json'), /not JSON/u);
  await refused(async dir => {
    for (const path of ['bin/chariox-kernel', 'bin/chariox', 'bin/chariox-app-package', `libexec/${INSTALLER_TOOL}`]) await writeFile(join(dir, path), UNIVERSAL);
  }, /arm64\+x86_64; build one package per architecture/u);
  await writeFile(fixture.output, '');
  await assert.rejects(packagePlan(parseArguments(args(fixture, ['--unsigned']), {})), /already exists/u);
});

test('the release bundle\'s darwin layout is accepted, and chariox-cli is packaged only when present', async t => {
  // release-bundle.mjs PLATFORMS['darwin-arm64']: bin/chariox, chariox-kernel, chariox-relay, chariox-app-package;
  // libexec/chariox-app-runtime-install; runtime/; share/chariox/slice-build-context; LICENSE and the signed manifest.
  const fixture = await bundle(t);
  await writeFile(join(fixture.dir, 'bin/chariox-relay'), Buffer.concat([ARM64, Buffer.from('relay')]), { mode: 0o755 });
  await rm(join(fixture.dir, 'SHA256SUMS'));
  for (const name of ['LICENSE', 'manifest.json', 'manifest.sig']) await writeFile(join(fixture.dir, name), 'not packaged\n');
  const plan = await packagePlan(parseArguments(args(fixture, ['--unsigned']), {}));
  assert.deepEqual(plan.release.binaries.map(binary => binary.name), ['chariox', 'chariox-app-package', 'chariox-kernel', 'chariox-relay']);
  assert.deepEqual(plan.release.ignored, ['LICENSE', 'manifest.json', 'manifest.sig']);
  assert.equal(plan.payload.find(entry => entry.path === `${PACKAGE_DIR}/bin.sha256`).content.split('\n').filter(Boolean)
    .map(line => line.split('  ')[1]).join(), '/usr/local/bin/chariox,/usr/local/bin/chariox-app-package,/usr/local/bin/chariox-kernel,/usr/local/bin/chariox-relay');
  // An older bundle's chariox-cli still installs beside it.
  await writeFile(join(fixture.dir, 'bin/chariox-cli'), Buffer.concat([ARM64, Buffer.from('cli')]), { mode: 0o755 });
  const older = await packagePlan(parseArguments(args(fixture, ['--unsigned']), {}));
  assert.ok(older.payload.some(entry => entry.path === 'usr/local/bin/chariox-cli' && entry.mode === 0o755));
  assert.match(older.payload.find(entry => entry.path === `${PACKAGE_DIR}/bin.sha256`).content, /  \/usr\/local\/bin\/chariox-cli\n/u);
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
  assert.match(text, new RegExp(`^# payload 0755 /${CONTEXT_DIR}/: the slice build context, 4 files, ${CONTEXT_BYTES} bytes, `
    + `listed in /${PACKAGE_DIR}/slice-build-context\\.sha256$`, 'mu'));
  assert.match(text, /^# payload 0444 \/usr\/local\/libexec\/chariox\/slice-build-context\.sha256$/mu);
  assert.doesNotMatch(text, /# payload \S+ \/usr\/local\/share\/chariox\/slice-build-context\/[^:]/u);
  const lines = text => text.split('\n').filter(Boolean).sort();
  assert.deepEqual(lines(plan.payload.find(entry => entry.path === `${PACKAGE_DIR}/slice-build-context.sha256`).content), lines(contextManifest()));
  const context = plan.payload.filter(entry => entry.path.startsWith(`${CONTEXT_DIR}/`));
  assert.deepEqual(Object.fromEntries(context.map(entry => [entry.path.slice(CONTEXT_DIR.length + 1), entry.mode])),
    Object.fromEntries(Object.keys(CONTEXT_FILES).map(path => [path, path === PROVISIONER ? 0o755 : 0o644])));
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

// The slice build context and its manifest as the payload lays them down; extra files are an earlier release's.
async function layContext(R, extra = {}) {
  for (const [path, text] of Object.entries({ ...CONTEXT_FILES, ...extra })) {
    await mkdir(dirname(join(R, CONTEXT_DIR, path)), { recursive: true });
    await writeFile(join(R, CONTEXT_DIR, path), text);
  }
  await mkdir(join(R, PACKAGE_DIR), { recursive: true });
  await writeFile(join(R, PACKAGE_DIR, 'slice-build-context.sha256'), contextManifest());
}

async function postinstall(fake, digest, inventory, { context = true, stale = {} } = {}) {
  if (context) await layContext(fake.R, stale);
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

test('postinstall leaves exactly the listed slice build context after an upgrade', darwin, async t => {
  const fake = await host(t);
  const digest = sha256('pinned\n');
  // An upgrade's payload does not remove the files a previous release installed.
  const result = await postinstall(fake, digest, 'pinned\n', { stale: { 'apps/kernel/src/bin/old.rs': 'fn main() {}\n', 'apps/old/README.md': 'old\n' } });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /removed 2 files an earlier release left in \S+\/usr\/local\/share\/chariox\/slice-build-context\n/u);
  const left = spawnSync('/usr/bin/find', ['.'], { cwd: join(fake.R, CONTEXT_DIR), encoding: 'utf8' }).stdout.trim().split('\n').sort();
  assert.deepEqual(left, ['.', './Cargo.toml', './apps', './apps/kernel', './apps/kernel/.charioxignore', './apps/kernel/slice-linux-docker',
    `./${PROVISIONER}`, './apps/kernel/slice-linux-docker/runtime-source-roots.txt'].sort());
  // A payload without its context is not this package's; nothing is enrolled.
  const missing = await host(t);
  const failure = await postinstall(missing, digest, 'pinned\n', { context: false });
  assert.equal(failure.status, 1);
  assert.match(failure.stderr, /the package's slice build context is missing/u);
  assert.deepEqual(missing.calls(), []);
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
  await file('usr/local/bin/chariox', 'replaced by the user');
  await file(`${PACKAGE_DIR}/start-kernel.sh`);
  await file(`${PACKAGE_DIR}/bin.sha256`, `${sha256('kernel')}  /usr/local/bin/chariox-kernel\n${sha256('cli')}  /usr/local/bin/chariox\n`);
  await layContext(fake.R);
  await mkdir(join(fake.R, 'usr/local/share/man'));
  const script = join(fake.R, PACKAGE_DIR, 'uninstall.sh');
  await writeFile(script, await renderTemplate('uninstall.sh', { ROOT: fake.R }), { mode: 0o755 });
  assert.equal(spawnSync(script, ['--force'], { encoding: 'utf8' }).status, 2);
  const dry = spawnSync(script, ['--dry-run'], { encoding: 'utf8' });
  assert.equal(dry.status, 0, dry.stderr);
  assert.match(dry.stdout, /would withdraw the App runtime enrollment/u);
  assert.match(dry.stdout, new RegExp(`would retire App runtime ${OLD}`, 'u'));
  assert.match(dry.stdout, /would remove the 4 unchanged files of \S+\/usr\/local\/share\/chariox\/slice-build-context\n/u);
  assert.equal(existsSync(join(fake.R, `${SUPPORT}/AppRuntime/runtime-enrollment.json`)), true);
  assert.equal(existsSync(join(fake.R, CONTEXT_DIR, PROVISIONER)), true);
  assert.equal(fake.calls().some(call => !call.startsWith('launchctl print') && !call.startsWith('pkgutil --pkg-info')), false);
  const result = spawnSync(script, [], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /kept \/usr\/local\/bin\/chariox: it changed/u);
  for (const gone of ['Library/LaunchAgents/dev.chariox.kernel.plist', SUPPORT, 'usr/local/bin/chariox-kernel', PACKAGE_DIR, 'usr/local/share/chariox'])
    assert.equal(existsSync(join(fake.R, gone)), false, gone);
  // /usr/local/share is shared with other software.
  assert.equal(existsSync(join(fake.R, 'usr/local/share/man')), true);
  assert.equal(readFileSync(join(fake.R, 'usr/local/bin/chariox'), 'utf8'), 'replaced by the user');
  assert.ok(fake.calls().includes(`chariox-app-runtime-install cleanup --inventory-sha256 ${OLD}`));
  assert.ok(fake.calls().includes('pkgutil --forget dev.chariox.pkg'));
});

test('uninstall keeps a changed file of the slice build context and refuses a manifest entry outside it', darwin, async t => {
  const fake = await host(t);
  await layContext(fake.R);
  await writeFile(join(fake.R, CONTEXT_DIR, 'Cargo.toml'), '[workspace]\n# edited\n');
  const manifest = join(fake.R, PACKAGE_DIR, 'slice-build-context.sha256');
  const script = join(fake.R, PACKAGE_DIR, 'uninstall.sh');
  await writeFile(script, await renderTemplate('uninstall.sh', { ROOT: fake.R }), { mode: 0o755 });
  await writeFile(manifest, `${contextManifest()}${sha256('kernel')}  /${CONTEXT_DIR}/../../../bin/chariox-kernel\n`);
  const refused = spawnSync(script, [], { encoding: 'utf8' });
  assert.equal(refused.status, 1);
  assert.match(refused.stderr, /unexpected entry in \S+slice-build-context\.sha256: \S+  \/usr\/local\/share\/chariox\/slice-build-context\/\.\.\/\.\.\/\.\.\/bin\/chariox-kernel/u);
  assert.equal(existsSync(join(fake.R, CONTEXT_DIR, PROVISIONER)), true);
  await writeFile(manifest, contextManifest());
  const result = spawnSync(script, [], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /kept \S+\/slice-build-context\/Cargo\.toml: it changed after the package installed it/u);
  assert.match(result.stdout, /remove the 3 unchanged files of /u);
  assert.match(result.stdout, /kept \S+\/usr\/local\/share\/chariox\/slice-build-context: it is not empty/u);
  const left = spawnSync('/usr/bin/find', ['.'], { cwd: join(fake.R, 'usr/local/share/chariox'), encoding: 'utf8' }).stdout.trim().split('\n').sort();
  assert.deepEqual(left, ['.', './slice-build-context', './slice-build-context/Cargo.toml']);
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

test('an unsigned package builds from a release bundle with the pinned postinstall', darwin, async t => {
  const fixture = await bundle(t);
  // The fixture installer is no real executable; the probe gets the macOS installer's answer to an ordinary user.
  const run = command => command[0].endsWith(INSTALLER_TOOL) ? { status: 1, stdout: '', stderr: 'app_runtime_installer_requires_root\n' }
    : spawnSync(command[0], command.slice(1), { encoding: 'utf8' });
  const { receipt } = await buildMacosPkg(parseArguments(args(fixture, ['--unsigned']), {}), { run });
  assert.equal(receipt.signing, 'unsigned test package: Gatekeeper refuses it');
  assert.equal(receipt.sha256, sha256(readFileSync(fixture.output)));
  assert.deepEqual(receipt.payload.map(entry => entry.path), ['/usr/local/bin/chariox', '/usr/local/bin/chariox-app-package',
    '/usr/local/bin/chariox-kernel', `/${PACKAGE_DIR}/chariox-app-runtime-install`, `/${PACKAGE_DIR}/start-kernel.sh`,
    `/${PACKAGE_DIR}/uninstall.sh`, `/${PACKAGE_DIR}/bin.sha256`, `/${PACKAGE_DIR}/slice-build-context.sha256`,
    '/Library/LaunchAgents/dev.chariox.kernel.plist']);
  assert.deepEqual(receipt.sliceBuildContext, { path: `/${CONTEXT_DIR}`, files: 4, bytes: CONTEXT_BYTES });
  assert.equal(existsSync(`${fixture.output}.build`), false);
  const expanded = join(fixture.root, 'expanded');
  assert.equal(spawnSync('/usr/sbin/pkgutil', ['--expand-full', fixture.output, expanded]).status, 0);
  assert.match(await readFile(join(expanded, 'chariox.pkg/Scripts/postinstall'), 'utf8'), new RegExp(`RUNTIME_DIGEST='${fixture.digest}'`, 'u'));
  // The installed slice build context: root's, beside /usr/local/bin, with the modes the kernel needs.
  const bom = spawnSync('/usr/bin/lsbom', ['-p', 'MUGf', join(expanded, 'chariox.pkg/Bom')], { encoding: 'utf8' }).stdout;
  for (const [path, mode] of [['usr/local/share', 'drwxr-xr-x'], ['usr/local/share/chariox', 'drwxr-xr-x'], [CONTEXT_DIR, 'drwxr-xr-x'],
    [`${CONTEXT_DIR}/${PROVISIONER}`, '-rwxr-xr-x'], [`${CONTEXT_DIR}/apps/kernel/.charioxignore`, '-rw-r--r--'],
    [`${PACKAGE_DIR}/slice-build-context.sha256`, '-r--r--r--']])
    assert.match(bom, new RegExp(`^${mode}\\s+root\\s+wheel\\s+\\./${path.replaceAll('.', '\\.')}$`, 'mu'), path);
  // Its manifest checks the delivered bytes.
  const payload = join(expanded, 'chariox.pkg/Payload');
  const manifest = await readFile(join(payload, PACKAGE_DIR, 'slice-build-context.sha256'), 'utf8');
  assert.deepEqual(manifest.split('\n').filter(Boolean).sort(), contextManifest().split('\n').filter(Boolean).sort());
  const check = spawnSync('/usr/bin/shasum', ['-a', '256', '-c', '-'], { input: manifest.replaceAll('  /', `  ${payload}/`), encoding: 'utf8' });
  assert.equal(check.status, 0, `${check.stdout}${check.stderr}`);
  assert.equal(check.stdout.match(/: OK$/gmu).length, 4);
  assert.equal(statSync(join(payload, CONTEXT_DIR, PROVISIONER)).mode & 0o777, 0o755);
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
