import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { cpSync, readFileSync, writeFileSync, appendFileSync } from 'node:fs';
import { lstat, mkdir, mkdtemp, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { formatPlan, parseArguments, releasePlan, signMacosRelease, STAPLE, SUBMISSION_ID, UsageError,
  WORKER_ENTITLEMENTS } from './sign-macos-release.mjs';

const SCRIPT = join(dirname(fileURLToPath(import.meta.url)), 'sign-macos-release.mjs');
const IDENTITY = 'Developer ID Application: Example Owner (ABCDE12345)';
const PROFILE = 'chariox-notary';
const MACH_O = Buffer.from([0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00, 0x00, 0x01]);

// Fake Mach-O headers exercise classification and the command plan only. No
// fixture is executed, signed or submitted.
async function fixture(t) {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'macos-release-signing-')));
  t.after(() => rm(root, { recursive: true, force: true }));
  const input = join(root, 'unsigned');
  await mkdir(join(input, 'sdk'), { recursive: true });
  for (const name of ['chariox-kernel', 'chariox-app-runtime-install', 'chariox-app-worker', 'chariox-setup', 'libnode.137.dylib', 'libchariox-app-runtime.dylib'])
    await writeFile(join(input, name), Buffer.concat([MACH_O, Buffer.from(name)]));
  await writeFile(join(input, 'sdk/index.js'), 'export {};\n');
  return { root, input, output: join(root, 'signed') };
}

const options = (paths, extra = []) => parseArguments(['--input', paths.input, '--output', paths.output,
  '--identity', IDENTITY, '--keychain-profile', PROFILE, ...extra], {});

test('arguments take the identity and profile from flags or the environment', () => {
  const parsed = parseArguments(['--input', 'a', '--output', 'b', '--dry-run'],
    { CHARIOX_CODESIGN_IDENTITY: IDENTITY, CHARIOX_NOTARY_PROFILE: PROFILE });
  assert.deepEqual(parsed, { input: 'a', output: 'b', dryRun: true, identity: IDENTITY, keychainProfile: PROFILE, teamId: 'ABCDE12345' });
  const flagged = parseArguments(['--input', 'a', '--output', 'b', '--identity', IDENTITY, '--keychain-profile', 'other'],
    { CHARIOX_CODESIGN_IDENTITY: 'Developer ID Application: Someone Else (ZZZZZ99999)', CHARIOX_NOTARY_PROFILE: PROFILE });
  assert.equal(flagged.identity, IDENTITY);
  assert.equal(flagged.keychainProfile, 'other');
  assert.equal(flagged.dryRun, false);
});

test('arguments refuse non-Developer ID identities, bad profiles and credential flags', () => {
  const base = ['--input', 'a', '--output', 'b'];
  for (const identity of ['-', 'Apple Development: Example Owner (ABCDE12345)', 'Developer ID Installer: Example Owner (ABCDE12345)',
    'Developer ID Application: Example Owner', '0123456789abcdef0123456789abcdef01234567', ''])
    assert.throws(() => parseArguments([...base, '--identity', identity, '--keychain-profile', PROFILE], {}), UsageError, identity);
  assert.throws(() => parseArguments([...base, '--identity', IDENTITY], {}), /keychain profile/u);
  assert.throws(() => parseArguments([...base, '--identity', IDENTITY, '--keychain-profile', 'bad profile'], {}), /keychain profile/u);
  assert.throws(() => parseArguments(['--input', 'a', '--identity', IDENTITY, '--keychain-profile', PROFILE], {}), /required/u);
  for (const flag of ['--password', '--apple-id', '--key', '--team-id'])
    assert.throws(() => parseArguments([...base, flag, 'secret-value'], {}), error =>
      error instanceof UsageError && error.message === `unsupported argument ${flag}`);
  assert.throws(() => parseArguments([...base, 'secret-value'], {}), error => !error.message.includes('secret-value'));
  assert.throws(() => parseArguments([...base, '--input', 'c'], {}), /duplicate --input/u);
  assert.throws(() => parseArguments([...base, '--dry-run', '--dry-run'], {}), /duplicate --dry-run/u);
  assert.throws(() => parseArguments(['--input', '--output', 'b'], {}), /--input needs a value/u);
});

test('MP-07 / MP-11: the plan grants JIT only to the App worker and Bun Setup', async t => {
  const paths = await fixture(t);
  const { plan } = await signMacosRelease(options(paths, ['--dry-run']),
    { run: () => assert.fail('dry run executed a command'), platform: 'linux' });
  await assert.rejects(lstat(paths.output), { code: 'ENOENT' });
  assert.deepEqual(plan.files.map(file => [file.path, file.role]), [
    ['chariox-app-runtime-install', 'executable'], ['chariox-app-worker', 'jit-executable'],
    ['chariox-kernel', 'executable'], ['chariox-setup', 'jit-executable'], ['libchariox-app-runtime.dylib', 'library'],
    ['libnode.137.dylib', 'library'], ['sdk/index.js', 'data']]);
  const signing = plan.steps.filter(step => step.phase === 'sign');
  assert.deepEqual(signing.map(step => step.file.path), ['libchariox-app-runtime.dylib', 'libnode.137.dylib',
    'chariox-app-runtime-install', 'chariox-app-worker', 'chariox-kernel', 'chariox-setup']);
  for (const step of signing) {
    assert.deepEqual(step.command.slice(0, 7), ['/usr/bin/codesign', '--force', '--sign', IDENTITY, '--timestamp', '--options', 'runtime']);
    assert.equal(step.command.at(-1), join(plan.output, step.file.path));
    const jit = ['chariox-app-worker', 'chariox-setup'].includes(step.file.path);
    assert.equal(step.command.includes('--entitlements'), jit);
    if (jit) assert.equal(step.command[8], WORKER_ENTITLEMENTS);
  }
  assert.equal(signing[3].command[8], WORKER_ENTITLEMENTS);
  assert.deepEqual(plan.steps.map(step => step.phase).filter((phase, index, all) => phase !== all[index - 1]),
    ['copy', 'sign', 'verify', 'describe', 'entitlements', 'archive', 'notarize', 'notary-log', 'gatekeeper']);
  assert.deepEqual(plan.steps.find(step => step.phase === 'notarize').command, ['/usr/bin/xcrun', 'notarytool', 'submit',
    `${plan.output}.notarization.zip`, '--keychain-profile', PROFILE, '--wait', '--output-format', 'json']);
  assert.deepEqual(plan.steps.filter(step => step.phase === 'gatekeeper').map(step => step.file.path),
    ['chariox-app-runtime-install', 'chariox-app-worker', 'chariox-kernel', 'chariox-setup']);
  assert.ok(plan.steps.every(step => step.phase === 'copy'
    || !step.command.some(word => word === paths.input || word.startsWith(`${paths.input}/`))));
  const text = formatPlan(plan);
  assert.match(text, /^# Dry run: nothing below was executed/u);
  assert.match(text, /\/usr\/bin\/codesign --force --sign 'Developer ID Application: Example Owner \(ABCDE12345\)' --timestamp/u);
  assert.match(text, new RegExp(`notarytool log '${SUBMISSION_ID}' --keychain-profile ${PROFILE}`, 'u'));
  assert.ok(text.includes(`# staple: ${STAPLE}`));
});

test('the entitlements file grants only the JIT', () => {
  const plist = readFileSync(WORKER_ENTITLEMENTS, 'utf8');
  assert.deepEqual([...plist.matchAll(/<key>([^<]+)<\/key>/gu)].map(match => match[1]), ['com.apple.security.cs.allow-jit']);
});

test('the plan refuses stale inventories, links, existing outputs and inputs without code', async t => {
  const paths = await fixture(t);
  await writeFile(join(paths.input, 'runtime-inventory.json'), '{}');
  await assert.rejects(releasePlan(options(paths)), /runtime-inventory\.json; sign the Chariox runtime inventory after/u);
  await rm(join(paths.input, 'runtime-inventory.json'));
  await symlink('/etc/hosts', join(paths.input, 'sdk/hosts'));
  await assert.rejects(releasePlan(options(paths)), /link or special file: sdk\/hosts/u);
  await rm(join(paths.input, 'sdk/hosts'));
  await assert.rejects(releasePlan({ ...options(paths), output: join(paths.input, 'signed') }), /outside --input/u);
  await mkdir(paths.output);
  await assert.rejects(releasePlan(options(paths)), /already exists/u);
  const empty = join(paths.root, 'empty');
  await mkdir(join(empty, 'sdk'), { recursive: true });
  await writeFile(join(empty, 'sdk/index.js'), '');
  await assert.rejects(releasePlan({ ...options(paths), input: empty, output: join(paths.root, 'other') }), /no Mach-O/u);
});

// Simulates the Apple tools: ditto copies, codesign changes the code bytes.
function apple({ notary = 'Accepted', runtime = true, authority = IDENTITY, extraEntitlement = false, gatekeeper = true,
  entitlementOverride = {} } = {}) {
  const calls = [];
  const signedEntitlements = new Map();
  const run = command => {
    calls.push(command);
    const [tool, ...args] = command;
    const file = args.at(-1);
    const ok = (stdout = '', stderr = '') => ({ status: 0, stdout, stderr });
    if (tool === '/usr/bin/ditto' && args[0] === '-c') { writeFileSync(file, 'zip'); return ok(); }
    if (tool === '/usr/bin/ditto') { cpSync(args[0], args[1], { recursive: true }); return ok(); }
    if (tool === '/bin/chmod') return ok();
    if (tool === '/usr/bin/codesign' && args[0] === '--force') {
      signedEntitlements.set(file, args.includes('--entitlements') ? readFileSync(args[args.indexOf('--entitlements') + 1], 'utf8') : '');
      appendFileSync(file, 'signature'); return ok();
    }
    if (tool === '/usr/bin/codesign' && args[0] === '--verify') return ok('', `${file}: valid on disk\n`);
    if (tool === '/usr/bin/codesign' && args.includes('--entitlements')) {
      const name = file.split('/').at(-1);
      let plist = entitlementOverride[name] ?? signedEntitlements.get(file) ?? '';
      if (extraEntitlement) plist += '<key>com.apple.security.cs.disable-library-validation</key><true/>';
      return ok(plist);
    }
    if (tool === '/usr/bin/codesign') return ok('', [`Executable=${file}`,
      `CodeDirectory v=20500 size=1 flags=${runtime ? '0x10000(runtime)' : '0x0(none)'} hashes=1+0 location=embedded`,
      `Authority=${authority}`, 'Authority=Developer ID Certification Authority', 'Authority=Apple Root CA',
      'Timestamp=1 Oct 2026 at 10:00:00', 'TeamIdentifier=ABCDE12345', ''].join('\n'));
    if (tool === '/usr/bin/xcrun' && args[1] === 'submit')
      return ok(JSON.stringify({ id: '2efe2717-52ef-43a5-96dc-0797e4ca1041', status: notary, message: 'Processing complete' }));
    if (tool === '/usr/bin/xcrun' && args[1] === 'log') { writeFileSync(file, '{}'); return ok(); }
    if (tool === '/usr/sbin/spctl') return gatekeeper ? ok('', `${file}: accepted\nsource=Notarized Developer ID\n`)
      : { status: 3, stdout: '', stderr: `${file}: rejected\n` };
    return { status: 1, stdout: '', stderr: `unexpected ${tool}` };
  };
  return { calls, run };
}

test('a signed run returns re-signed digests, the notarization and Gatekeeper results', async t => {
  const paths = await fixture(t);
  const tools = apple();
  const { receipt } = await signMacosRelease(options(paths), { run: tools.run, platform: 'darwin' });
  assert.equal(receipt.schema, 'chariox.macos-release-signing.v1');
  assert.equal(receipt.teamId, 'ABCDE12345');
  assert.equal(receipt.runtimeInventory, 'not-signed');
  assert.equal(receipt.staple, STAPLE);
  for (const file of receipt.files) {
    assert.equal(file.sha256 === file.unsignedSha256, file.role === 'data', file.path);
    assert.deepEqual(file.entitlements, file.role === 'jit-executable' ? ['com.apple.security.cs.allow-jit'] : []);
  }
  assert.deepEqual(receipt.notarization, { submissionId: '2efe2717-52ef-43a5-96dc-0797e4ca1041', status: 'Accepted',
    archive: `${receipt.output}.notarization.zip`, archiveSha256: receipt.notarization.archiveSha256,
    log: `${receipt.output}.notarization-log.json` });
  assert.deepEqual(tools.calls.find(command => command[2] === 'log').slice(3, 4), ['2efe2717-52ef-43a5-96dc-0797e4ca1041']);
  assert.deepEqual(receipt.gatekeeper.map(entry => entry.path), ['chariox-app-runtime-install', 'chariox-app-worker', 'chariox-kernel', 'chariox-setup']);
  assert.deepEqual(receipt.files.find(file => file.path === 'chariox-setup').entitlements, ['com.apple.security.cs.allow-jit']);
  assert.equal(readFileSync(join(paths.input, 'chariox-kernel')).toString('latin1').endsWith('signature'), false);
});

test('MP-07 / MP-11: Setup requires enabled JIT and refuses excess entitlements', async t => {
  const paths = await fixture(t);
  for (const plist of ['', '<plist><dict><key>com.apple.security.cs.allow-jit</key><false/></dict></plist>',
    '<plist><dict><key>com.apple.security.cs.allow-jit</key><true/><key>com.apple.security.cs.disable-library-validation</key><true/></dict></plist>']) {
    await assert.rejects(signMacosRelease(options(paths), {
      run: apple({ entitlementOverride: { 'chariox-setup': plist } }).run, platform: 'darwin',
    }), /chariox-setup has unexpected entitlements/u);
    await assert.rejects(lstat(paths.output), { code: 'ENOENT' });
  }
  await assert.rejects(signMacosRelease(options(paths), {
    run: apple({ entitlementOverride: { 'chariox-kernel': '<plist><dict><key>com.apple.security.cs.allow-jit</key><true/></dict></plist>' } }).run,
    platform: 'darwin',
  }), /chariox-kernel has unexpected entitlements/u);
});

test('MP-07 / MP-11: release workflow executes signed Setup and rejects broken or wrong-version output', async t => {
  const workflow = readFileSync(new URL('../.github/workflows/release-bundle.yml', import.meta.url), 'utf8');
  const smoke = workflow.split('\n').find(line => line.includes('signed/bin/chariox-setup') && line.trimStart().startsWith('test '));
  assert.ok(smoke, 'macOS release must smoke-test the signed Setup executable');
  assert.ok(workflow.indexOf(smoke) > workflow.indexOf('node scripts/sign-macos-release.mjs --input "$RUNNER_TEMP/executables"'));
  assert.ok(workflow.indexOf(smoke) < workflow.indexOf('- name: Assemble, sign and verify the bundle', workflow.indexOf('  macos:')));
  const paths = await fixture(t);
  await mkdir(join(paths.root, 'signed/bin'), { recursive: true });
  const setup = join(paths.root, 'signed/bin/chariox-setup');
  for (const [body, accepted] of [['echo "Chariox Setup 0.3.0"', true], ['echo "Chariox Setup 0.2.0"', false], ['exit 1', false]]) {
    await writeFile(setup, `#!/bin/sh\n${body}\n`, { mode: 0o755 });
    const result = spawnSync('/bin/bash', ['-c', smoke.trim()], { encoding: 'utf8',
      env: { RUNNER_TEMP: paths.root, VERSION: '0.3.0' } });
    assert.equal(result.status === 0, accepted, result.stderr);
  }
});

test('a rejected notarization or a wrong signature removes the signed copy', async t => {
  const paths = await fixture(t);
  await assert.rejects(signMacosRelease(options(paths), { run: apple({ notary: 'Invalid' }).run, platform: 'darwin' }),
    /notarization Invalid; inspect it with: xcrun notarytool log 2efe2717-52ef-43a5-96dc-0797e4ca1041 --keychain-profile chariox-notary/u);
  await assert.rejects(lstat(paths.output), { code: 'ENOENT' });
  await assert.rejects(lstat(`${paths.output}.notarization.zip`), { code: 'ENOENT' });
  await assert.rejects(signMacosRelease(options(paths), { run: apple({ runtime: false }).run, platform: 'darwin' }), /hardened runtime/u);
  await assert.rejects(signMacosRelease(options(paths),
    { run: apple({ authority: 'Apple Development: Example Owner (ABCDE12345)' }).run, platform: 'darwin' }), /named Developer ID identity/u);
  await assert.rejects(signMacosRelease(options(paths), { run: apple({ extraEntitlement: true }).run, platform: 'darwin' }),
    /unexpected entitlements/u);
  await assert.rejects(signMacosRelease(options(paths), { run: apple().run, platform: 'linux' }), /runs on macOS/u);
  await assert.rejects(lstat(paths.output), { code: 'ENOENT' });
});

test('a failure after the notary log is written leaves nothing that blocks a retry', async t => {
  const paths = await fixture(t);
  await assert.rejects(signMacosRelease(options(paths), { run: apple({ gatekeeper: false }).run, platform: 'darwin' }),
    /gatekeeper failed for chariox-app-runtime-install.*; accepted notarization 2efe2717-52ef-43a5-96dc-0797e4ca1041 keeps its log: xcrun notarytool log 2efe2717-52ef-43a5-96dc-0797e4ca1041 --keychain-profile chariox-notary$/su);
  for (const path of [paths.output, `${paths.output}.notarization.zip`, `${paths.output}.notarization-log.json`])
    await assert.rejects(lstat(path), { code: 'ENOENT' }, path);
  const { receipt } = await signMacosRelease(options(paths), { run: apple().run, platform: 'darwin' });
  assert.equal(receipt.notarization.log, `${receipt.output}.notarization-log.json`);
});

test('the command line prints the dry-run plan and never echoes a misplaced value', async t => {
  const paths = await fixture(t);
  const cli = args => spawnSync(process.execPath, [SCRIPT, ...args], { encoding: 'utf8', env: { PATH: process.env.PATH } });
  const plan = cli(['--dry-run', '--input', paths.input, '--output', paths.output, '--identity', IDENTITY, '--keychain-profile', PROFILE]);
  assert.equal(plan.status, 0, plan.stderr);
  assert.match(plan.stdout, /notarytool submit .*--keychain-profile chariox-notary --wait/u);
  await assert.rejects(lstat(paths.output), { code: 'ENOENT' });
  const misplaced = cli(['--input', paths.input, '--output', paths.output, '--password', 'hunter2-secret']);
  assert.equal(misplaced.status, 2);
  assert.equal(misplaced.stderr, 'macos_release_signing_failed: unsupported argument --password\n');
});
