#!/usr/bin/env node
// macOS Developer ID signing for Chariox release artifacts: the kernel, its
// helpers, and the App runtime's native worker and libraries. The owner names
// the signing identity and the notarytool keychain profile at run time. This
// tool never creates, stores or reads credentials, identities or profiles.
// It signs a copy (the unsigned input is kept), notarizes it, and verifies the
// signatures and the Gatekeeper assessment. Code signing changes bytes, so the
// Chariox runtime inventory is signed over this tool's output, never before it.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { lstat, open, readdir, readFile, realpath, rm } from 'node:fs/promises';
import { basename, dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const WORKER_ENTITLEMENTS = join(dirname(fileURLToPath(import.meta.url)), 'macos-app-worker.entitlements');
const JIT_ENTITLEMENT = 'com.apple.security.cs.allow-jit';
// MP-07 / MP-11: V8 in the App worker and JavaScriptCore in Bun Setup require JIT.
// Other artifacts run without entitlements.
const JIT_EXECUTABLES = new Set(['chariox-app-worker', 'chariox-setup']);
// A Chariox inventory signed before platform signing describes the wrong bytes.
const STALE = new Set(['runtime-inventory.json', 'runtime-inventory.sig', '.runtime-lease']);
const MACH_O = new Set(['feedface', 'feedfacf', 'cefaedfe', 'cffaedfe', 'cafebabe', 'bebafeca']);
const IDENTITY = /^Developer ID Application: [^\r\n]+ \(([A-Z0-9]{10})\)$/u;
const PROFILE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/u;
const FILE_LIMIT = 256;
const TOOLS = { chmod: '/bin/chmod', codesign: '/usr/bin/codesign', ditto: '/usr/bin/ditto',
  spctl: '/usr/sbin/spctl', xcrun: '/usr/bin/xcrun' };
export const SUBMISSION_ID = '<submission-id>';
export const STAPLE = 'not-applicable: bare Mach-O files and zip archives cannot hold a stapled ticket';

export class UsageError extends Error {}

const FLAGS = new Map([['--input', 'input'], ['--output', 'output'],
  ['--identity', 'identity'], ['--keychain-profile', 'keychainProfile']]);

export function parseArguments(argv, env = process.env) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (flag === '--dry-run') {
      if (options.dryRun) throw new UsageError('duplicate --dry-run');
      options.dryRun = true;
      continue;
    }
    const key = FLAGS.get(flag);
    // Never echo an unknown value: it may be a credential typed in the wrong place.
    if (!key) throw new UsageError(flag.startsWith('--') ? `unsupported argument ${flag.split('=')[0]}` : 'unexpected positional argument');
    if (Object.hasOwn(options, key)) throw new UsageError(`duplicate ${flag}`);
    const value = argv[index + 1];
    if (!value || value.startsWith('--')) throw new UsageError(`${flag} needs a value`);
    options[key] = value;
    index += 1;
  }
  options.dryRun ??= false;
  options.identity ??= env.CHARIOX_CODESIGN_IDENTITY;
  options.keychainProfile ??= env.CHARIOX_NOTARY_PROFILE;
  if (!options.input || !options.output) throw new UsageError('--input and --output are required');
  const identity = IDENTITY.exec(options.identity ?? '');
  if (!identity) throw new UsageError('--identity or CHARIOX_CODESIGN_IDENTITY must be "Developer ID Application: <Name> (<TEAMID>)"');
  if (!PROFILE.test(options.keychainProfile ?? ''))
    throw new UsageError('--keychain-profile or CHARIOX_NOTARY_PROFILE must name a notarytool keychain profile');
  return { ...options, teamId: identity[1] };
}

const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');

async function header(path) {
  const file = await open(path, 'r');
  try {
    const buffer = Buffer.alloc(4);
    const { bytesRead } = await file.read(buffer, 0, 4, 0);
    return bytesRead === 4 ? buffer.toString('hex') : '';
  } finally { await file.close(); }
}

function role(path, magic) {
  if (!MACH_O.has(magic)) return 'data';
  if (path.endsWith('.dylib')) return 'library';
  return JIT_EXECUTABLES.has(basename(path)) ? 'jit-executable' : 'executable';
}

async function artifacts(input) {
  const files = [];
  async function walk(directory, depth) {
    if (depth > 8) throw new Error('input nesting is too deep');
    const names = (await readdir(directory)).sort();
    for (const name of names) {
      const path = join(directory, name);
      const metadata = await lstat(path);
      const relativePath = relative(input, path);
      if (metadata.isDirectory()) { await walk(path, depth + 1); continue; }
      if (!metadata.isFile()) throw new Error(`input contains a link or special file: ${relativePath}`);
      if (STALE.has(name)) throw new Error(`input contains ${relativePath}; sign the Chariox runtime inventory after platform signing`);
      if (files.length === FILE_LIMIT) throw new Error(`input has more than ${FILE_LIMIT} files`);
      files.push({ path: relativePath, role: role(relativePath, await header(path)),
        size: metadata.size, unsignedSha256: sha256(await readFile(path)) });
    }
  }
  await walk(input, 0);
  if (!files.some(file => file.role !== 'data')) throw new Error('input has no Mach-O artifacts');
  return files;
}

const absent = async path => !(await lstat(path).catch(() => null));

export async function releasePlan({ input, output, identity, keychainProfile }) {
  input = await realpath(input);
  if (!(await lstat(input)).isDirectory()) throw new Error('--input must be a directory');
  output = join(await realpath(dirname(resolve(output))), basename(resolve(output)));
  if (output === input || output.startsWith(`${input}${sep}`) || input.startsWith(`${output}${sep}`))
    throw new Error('--output must be outside --input');
  const archive = `${output}.notarization.zip`;
  const log = `${output}.notarization-log.json`;
  for (const path of [output, archive, log]) if (!(await absent(path))) throw new Error(`${path} already exists`);
  const files = await artifacts(input);
  // Inside-out: libraries before the executables that load them.
  const code = [...files.filter(file => file.role === 'library'), ...files.filter(file => file.role.endsWith('executable'))];
  const target = file => join(output, file.path);
  const steps = [
    { phase: 'copy', command: [TOOLS.ditto, input, output] },
    { phase: 'copy', command: [TOOLS.chmod, '-R', 'u+w', output] },
    ...code.map(file => ({ phase: 'sign', file, command: [TOOLS.codesign, '--force', '--sign', identity,
      '--timestamp', '--options', 'runtime',
      ...(file.role === 'jit-executable' ? ['--entitlements', WORKER_ENTITLEMENTS] : []), target(file)] })),
    ...code.map(file => ({ phase: 'verify', file, command: [TOOLS.codesign, '--verify', '--strict', '--verbose=2', target(file)] })),
    ...code.map(file => ({ phase: 'describe', file, command: [TOOLS.codesign, '--display', '--verbose=2', target(file)] })),
    ...code.map(file => ({ phase: 'entitlements', file, command: [TOOLS.codesign, '--display', '--entitlements', '-', '--xml', target(file)] })),
    { phase: 'archive', command: [TOOLS.ditto, '-c', '-k', '--sequesterRsrc', '--keepParent', output, archive] },
    { phase: 'notarize', command: [TOOLS.xcrun, 'notarytool', 'submit', archive, '--keychain-profile', keychainProfile, '--wait', '--output-format', 'json'] },
    { phase: 'notary-log', command: [TOOLS.xcrun, 'notarytool', 'log', SUBMISSION_ID, '--keychain-profile', keychainProfile, log] },
    // `--type exec` rejects code outside an app bundle; `install` assesses a bare tool's notarization.
    ...code.filter(file => file.role !== 'library').map(file => ({ phase: 'gatekeeper', file,
      command: [TOOLS.spctl, '--assess', '--type', 'install', '--verbose=2', target(file)] })),
  ];
  return { input, output, archive, log, files, steps };
}

const quote = word => /^[A-Za-z0-9_/.,:=@%+-]+$/u.test(word) ? word : `'${word.replaceAll("'", "'\\''")}'`;

export function formatPlan(plan) {
  const lines = ['# Dry run: nothing below was executed. Unsigned input is never modified.'];
  for (const file of plan.files) lines.push(`# ${file.role.padEnd(14)} ${file.unsignedSha256}  ${file.path}`);
  let phase = '';
  for (const step of plan.steps) {
    if (step.phase !== phase) lines.push(`# ${(phase = step.phase)}`);
    lines.push(step.command.map(quote).join(' '));
  }
  lines.push(`# staple: ${STAPLE}`);
  lines.push('# next: sign the Chariox runtime inventory over the signed output bytes');
  return `${lines.join('\n')}\n`;
}

function runCommand(command) {
  const notarize = command[1] === 'notarytool' && command[2] === 'submit';
  const result = spawnSync(command[0], command.slice(1), { encoding: 'utf8', maxBuffer: 16 * 1024 * 1024,
    timeout: notarize ? 2 * 60 * 60 * 1000 : 10 * 60 * 1000, stdio: ['ignore', 'pipe', 'pipe'] });
  return { status: result.status, stdout: result.stdout ?? '', stderr: result.stderr ?? result.error?.message ?? '' };
}

function checked(run, step, command = step.command) {
  const result = run(command);
  if (result.status !== 0) {
    const detail = `${result.stderr}${result.stdout}`.trim().slice(-2000);
    throw new Error(`${step.phase} failed${step.file ? ` for ${step.file.path}` : ''}${detail ? `: ${detail}` : ''}`);
  }
  return result;
}

function describe(result, step, { identity, teamId }) {
  const text = `${result.stderr}\n${result.stdout}`;
  const lines = new Set(text.split('\n').map(line => line.trim()));
  if (!lines.has(`Authority=${identity}`)) throw new Error(`${step.file.path} is not signed by the named Developer ID identity`);
  if (!lines.has(`TeamIdentifier=${teamId}`)) throw new Error(`${step.file.path} has the wrong team identifier`);
  if (![...lines].some(line => line.startsWith('Timestamp='))) throw new Error(`${step.file.path} has no secure timestamp`);
  if (!/flags=0x[0-9a-f]+\([^)]*\bruntime\b[^)]*\)/u.test(text)) throw new Error(`${step.file.path} lacks the hardened runtime`);
}

function entitlements(result, step) {
  const jit = step.file.role === 'jit-executable';
  const keys = [...result.stdout.matchAll(/<key>([^<]+)<\/key>/gu)].map(match => match[1]);
  const enabled = /<key>com\.apple\.security\.cs\.allow-jit<\/key>\s*<true\s*\/>/u.test(result.stdout);
  if (jit ? keys.length !== 1 || keys[0] !== JIT_ENTITLEMENT || !enabled : keys.length)
    throw new Error(`${step.file.path} has unexpected entitlements: ${keys.join(', ') || 'none'}`);
}

function notarization(result, profile) {
  let body;
  try { body = JSON.parse(result.stdout); } catch { throw new Error('notarytool returned no JSON result'); }
  if (typeof body?.id !== 'string' || !/^[A-Za-z0-9-]{1,64}$/u.test(body.id)) throw new Error('notarytool returned no submission id');
  if (body.status !== 'Accepted')
    throw new Error(`notarization ${body.status}; inspect it with: xcrun notarytool log ${body.id} --keychain-profile ${profile}`);
  return body.id;
}

export async function signMacosRelease(options, { run = runCommand, platform = process.platform } = {}) {
  const plan = await releasePlan(options);
  if (options.dryRun) return { plan };
  if (platform !== 'darwin') throw new Error('macOS release signing runs on macOS');
  let copied = false;
  let submissionId;
  try {
    let log = plan.log;
    const gatekeeper = [];
    for (const step of plan.steps) {
      if (step.phase === 'notary-log') {
        // The log is evidence; an accepted submission stands without it.
        const command = step.command.map(word => word === SUBMISSION_ID ? submissionId : word);
        if (run(command).status !== 0) log = 'unavailable';
        continue;
      }
      // A partial copy is still ours to remove.
      if (step.phase === 'copy') copied = true;
      const result = checked(run, step);
      if (step.phase === 'describe') describe(result, step, options);
      else if (step.phase === 'entitlements') entitlements(result, step);
      else if (step.phase === 'notarize') submissionId = notarization(result, options.keychainProfile);
      else if (step.phase === 'gatekeeper') {
        if (!/source=Notarized Developer ID/u.test(`${result.stderr}\n${result.stdout}`))
          throw new Error(`${step.file.path} is not accepted as notarized Developer ID code`);
        gatekeeper.push({ path: step.file.path, assessment: 'accepted', source: 'Notarized Developer ID' });
      }
    }
    const files = [];
    for (const file of plan.files) {
      const bytes = await readFile(join(plan.output, file.path));
      const signed = sha256(bytes);
      if ((file.role === 'data') !== (signed === file.unsignedSha256))
        throw new Error(`${file.path} ${file.role === 'data' ? 'changed during signing' : 'was not re-signed'}`);
      files.push({ ...file, size: bytes.length, sha256: signed,
        entitlements: file.role === 'jit-executable' ? [JIT_ENTITLEMENT] : [] });
    }
    return { plan, receipt: {
      schema: 'chariox.macos-release-signing.v1', identity: options.identity, teamId: options.teamId,
      keychainProfile: options.keychainProfile, input: plan.input, output: plan.output, files,
      notarization: { submissionId, status: 'Accepted', archive: plan.archive,
        archiveSha256: sha256(await readFile(plan.archive)), log },
      staple: STAPLE, gatekeeper, runtimeInventory: 'not-signed',
    } };
  } catch (error) {
    // The plan proved all three paths absent, so a failed run removes them and a retry can reuse them.
    if (copied) await Promise.all([plan.output, plan.archive, plan.log].map(path => rm(path, { recursive: true, force: true })));
    if (submissionId) error.message += `; accepted notarization ${submissionId} keeps its log: `
      + `xcrun notarytool log ${submissionId} --keychain-profile ${options.keychainProfile}`;
    throw error;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const options = parseArguments(process.argv.slice(2));
    const { plan, receipt } = await signMacosRelease(options);
    process.stdout.write(options.dryRun ? formatPlan(plan) : `${JSON.stringify(receipt, null, 2)}\n`);
  } catch (error) {
    process.stderr.write(`macos_release_signing_failed: ${error.message}\n`);
    process.exitCode = error instanceof UsageError ? 2 : 1;
  }
}
