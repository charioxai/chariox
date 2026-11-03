#!/usr/bin/env node
// Build a macOS package from release binaries and their slice context.
// The WASM App worker ships and is signed beside the kernel. Only system
// packaging/signing tools run during planning and assembly.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { constants, createReadStream } from 'node:fs';
import { chmod, copyFile, lstat, mkdir, open, readdir, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { basename, dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const IDENTIFIER = 'dev.chariox.pkg';
export const AGENT_LABEL = 'dev.chariox.kernel';
export const MIN_MACOS = '13.5';
export const COMPONENT = 'chariox.pkg';
export const SUBMISSION_ID = '<submission-id>';
export const TEMPLATES = join(dirname(fileURLToPath(import.meta.url)), 'macos-pkg');
export const PACKAGE_DIR = 'usr/local/libexec/chariox';
const DEPLOY = join(dirname(TEMPLATES), '../deploy');
export const ADMISSION_LABEL = 'dev.chariox.docker-admission-locks';
export const ADMISSION_CONTRACT = 'chariox.docker-admission-locks.read-only.v1';
const SLICE_CONTEXT = 'share/chariox/slice-build-context';
// Beside usr/local/bin, where a release kernel looks for it.
export const CONTEXT_DIR = `usr/local/${SLICE_CONTEXT}`;
const SLICE_PROVISIONER = 'apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh';
// The context paths the manifest may hold (uninstall.sh checks the same): no
// spaces, quotes or line breaks, and no . or .. component.
const CONTEXT_PATH = /^\.?[A-Za-z0-9_+-][A-Za-z0-9._+-]*(\/\.?[A-Za-z0-9_+-][A-Za-z0-9._+-]*)*$/u;
const BINARIES = new Map([['chariox-kernel', true], ['chariox', true], ['chariox-app-package', false],
  ['chariox-app-wasm-worker', true], ['chariox-relay', false], ['chariox-cli', false]]);
const IDENTITY = /^Developer ID Installer: [^\r\n]+ \(([A-Z0-9]{10})\)$/u;
const PROFILE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/u;
const HEX64 = /^[0-9a-f]{64}$/u;
const VERSION = /^\d{1,6}(\.\d{1,6}){0,3}$/u;
const CPU = new Map([[0x0100000c, 'arm64'], [0x01000007, 'x86_64']]);
// Static marker compiled into supported installers. Bundle programs never run here.
const TOOLS = { codesign: '/usr/bin/codesign', pkgbuild: '/usr/bin/pkgbuild', productbuild: '/usr/bin/productbuild',
  productsign: '/usr/bin/productsign', pkgutil: '/usr/sbin/pkgutil', spctl: '/usr/sbin/spctl', xcrun: '/usr/bin/xcrun' };

export class UsageError extends Error {}

const FLAGS = new Map([['--bundle', 'bundle'], ['--output', 'output'], ['--version', 'version'],
  ['--identity', 'identity'], ['--keychain-profile', 'keychainProfile']]);
const SWITCHES = new Map([['--dry-run', 'dryRun'], ['--unsigned', 'unsigned']]);

export function parseArguments(argv, env = process.env) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    const toggle = SWITCHES.get(flag);
    if (toggle) {
      if (options[toggle]) throw new UsageError(`duplicate ${flag}`);
      options[toggle] = true;
      continue;
    }
    const key = FLAGS.get(flag);
    // Never echo an unknown value: it may be a credential typed in the wrong place.
    if (!key) throw new UsageError(flag.startsWith('--') ? `unsupported argument ${flag.split(/[=\s]/u)[0]}` : 'unexpected positional argument');
    if (Object.hasOwn(options, key)) throw new UsageError(`duplicate ${flag}`);
    const value = argv[index + 1];
    if (!value || value.startsWith('--')) throw new UsageError(`${flag} needs a value`);
    options[key] = value;
    index += 1;
  }
  options.dryRun ??= false;
  options.unsigned ??= false;
  if (!options.bundle || !options.output || !options.version) throw new UsageError('--bundle, --output and --version are required');
  if (!options.output.endsWith('.pkg')) throw new UsageError('--output must name a .pkg file');
  if (!VERSION.test(options.version)) throw new UsageError('--version must be numeric, such as 1.2.3');
  if (options.unsigned) {
    if (options.identity || options.keychainProfile) throw new UsageError('--unsigned takes no --identity or --keychain-profile');
    return { ...options, identity: null, keychainProfile: null, teamId: null };
  }
  options.identity ??= env.CHARIOX_INSTALLER_IDENTITY;
  options.keychainProfile ??= env.CHARIOX_NOTARY_PROFILE;
  const identity = IDENTITY.exec(options.identity ?? '');
  if (!identity) throw new UsageError('--identity or CHARIOX_INSTALLER_IDENTITY must be "Developer ID Installer: <Name> (<TEAMID>)"; --unsigned builds a test package');
  if (!PROFILE.test(options.keychainProfile ?? ''))
    throw new UsageError('--keychain-profile or CHARIOX_NOTARY_PROFILE must name a notarytool keychain profile');
  return { ...options, teamId: identity[1] };
}

const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');

async function hashFile(path) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest('hex');
}

// The architectures of a Mach-O file, or null for any other file.
async function machO(path) {
  const file = await open(path, 'r');
  try {
    const header = Buffer.alloc(4096);
    const { bytesRead } = await file.read(header, 0, header.length, 0);
    if (bytesRead < 8) return null;
    const magic = header.readUInt32BE(0);
    if (magic === 0xcffaedfe) return [CPU.get(header.readUInt32LE(4)) ?? 'unsupported'];
    if (magic === 0xcafebabe || magic === 0xcafebabf) {
      const count = header.readUInt32BE(4);
      const size = magic === 0xcafebabe ? 20 : 32;
      if (count < 1 || 8 + count * size > bytesRead) return ['unsupported'];
      return Array.from({ length: count }, (_, slice) => CPU.get(header.readUInt32BE(8 + slice * size)) ?? 'unsupported').sort();
    }
    return [0xfeedface, 0xcefaedfe, 0xfeedfacf].includes(magic) ? ['unsupported'] : null;
  } finally { await file.close(); }
}

async function executable(path, label) {
  const metadata = await lstat(path).catch(() => null);
  if (!metadata?.isFile()) throw new Error(`${label} is ${metadata ? 'not a regular file' : 'missing'}`);
  const archs = await machO(path);
  if (!archs) throw new Error(`${label} is not a Mach-O executable`);
  if (archs.includes('unsupported')) throw new Error(`${label} has an unsupported architecture`);
  return { path, archs, sha256: await hashFile(path) };
}

// Every regular file below a bundle directory, sorted, within its limits; links and special files are refused.
async function bundleFiles(bundle, label, { maxFiles = Infinity, maxBytes = Infinity, maxDepth = 16 } = {}) {
  const root = join(bundle, label);
  const files = [];
  let bytes = 0;
  async function walk(directory, depth) {
    if (depth > maxDepth) throw new Error(`${label}/ nests too deeply`);
    for (const name of (await readdir(directory)).sort()) {
      const path = join(directory, name);
      const metadata = await lstat(path);
      if (metadata.isDirectory()) { await walk(path, depth + 1); continue; }
      if (!metadata.isFile()) throw new Error(`${label}/${relative(root, path)} is a link or special file`);
      if (files.length === maxFiles) throw new Error(`${label}/ has more than ${maxFiles} files`);
      if ((bytes += metadata.size) > maxBytes) throw new Error(`${label}/ exceeds ${maxBytes / 1024 / 1024} MiB`);
      files.push({ path: relative(root, path), source: path, mode: metadata.mode });
    }
  }
  await walk(root, 0);
  return { path: root, files, bytes };
}


async function contextFiles(bundle, kernel) {
  // Only a release kernel with that lookup finds it (the path is a string constant of the lookup); an older
  // or debug kernel would look in /usr/lib, which macOS does not let a package write, or in its build checkout.
  const kernelBytes = await readFile(kernel);
  if (!kernelBytes.includes(SLICE_CONTEXT))
    throw new Error(`bin/chariox-kernel does not look for ${SLICE_CONTEXT} beside its bin/; build a release kernel that has that lookup`);
  if (!kernelBytes.includes(ADMISSION_CONTRACT))
    throw new Error('bin/chariox-kernel cannot use provisioned root-owned read-only Docker admission locks; build a release kernel with the admission-lock contract from #715 before packaging');
  const provisioner = await lstat(join(bundle, SLICE_CONTEXT, SLICE_PROVISIONER)).catch(() => null);
  if (!provisioner?.isFile() || !(provisioner.mode & 0o100))
    throw new Error(`the bundle has no executable ${SLICE_CONTEXT}/${SLICE_PROVISIONER}: a release kernel runs local Docker slices from that slice build context`);
  if ((await readdir(join(bundle, 'share'))).join('/') !== 'chariox' || (await readdir(join(bundle, 'share/chariox'))).join('/') !== 'slice-build-context')
    throw new Error('share/ must hold only chariox/slice-build-context');
  const context = await bundleFiles(bundle, SLICE_CONTEXT);
  for (const file of context.files) {
    if (!CONTEXT_PATH.test(file.path)) throw new Error(`${SLICE_CONTEXT}/ has a path the package cannot list: ${JSON.stringify(file.path)}`);
    file.mode = file.mode & 0o111 ? 0o755 : 0o644;
    file.sha256 = await hashFile(file.source);
  }
  return context;
}

export async function readBundle(input) {
  const bundle = await realpath(input);
  if (!(await lstat(bundle)).isDirectory()) throw new Error('--bundle must be a directory');
  const top = (await readdir(bundle)).sort();
  for (const name of ['bin'])
    if (!(await lstat(join(bundle, name)).catch(() => null))?.isDirectory()) throw new Error(`the bundle has no ${name}/ directory`);
  const binaries = [];
  for (const name of (await readdir(join(bundle, 'bin'))).sort()) {
    if (!BINARIES.has(name)) throw new Error(`bin/${name} is not a Chariox release binary`);
    binaries.push({ name, ...(await executable(join(bundle, 'bin', name), `bin/${name}`)) });
  }
  for (const [name, required] of BINARIES)
    if (required && !binaries.some(binary => binary.name === name)) throw new Error(`the bundle has no bin/${name}`);
  const { archs } = binaries.find(binary => binary.name === 'chariox-kernel');
  for (const [label, item] of binaries.map(binary => [`bin/${binary.name}`, binary]))
    if (item.archs.join() !== archs.join()) throw new Error(`${label} is ${item.archs.join('+')}, but bin/chariox-kernel is ${archs.join('+')}`);
  if (archs.length !== 1) throw new Error(`bin/chariox-kernel is ${archs.join('+')}; build one package per architecture`);
  const context = await contextFiles(bundle, binaries.find(binary => binary.name === 'chariox-kernel').path);
  return { bundle, binaries, context, archs, ignored: top.filter(name => !['bin', 'share'].includes(name)) };
}

// Templates name host paths below @@ROOT@@, which is empty in the package.
export async function renderTemplate(name, values) {
  for (const value of Object.values(values))
    if (/['\n\r]/u.test(value)) throw new Error(`${name}: a template value may not hold quotes or line breaks`);
  return (await readFile(join(TEMPLATES, name), 'utf8')).replace(/@@([A-Z_]+)@@/gu, (_, key) => {
    if (!Object.hasOwn(values, key)) throw new Error(`${name} needs ${key}`);
    return values[key];
  });
}

export function distributionXml({ version, archs }) {
  return `<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
    <title>Chariox</title>
    <product id="dev.chariox" version="${version}"/>
    <options customize="never" require-scripts="true" hostArchitectures="${archs.join(',')}"/>
    <domains enable_anywhere="false" enable_currentUserHome="false" enable_localSystem="true"/>
    <volume-check>
        <allowed-os-versions>
            <os-version min="${MIN_MACOS}"/>
        </allowed-os-versions>
    </volume-check>
    <choices-outline>
        <line choice="${IDENTIFIER}"/>
    </choices-outline>
    <choice id="${IDENTIFIER}" visible="false" title="Chariox">
        <pkg-ref id="${IDENTIFIER}"/>
    </choice>
    <pkg-ref id="${IDENTIFIER}" version="${version}" onConclusion="none">${COMPONENT}</pkg-ref>
</installer-gui-script>
`;
}

const absent = async path => !(await lstat(path).catch(() => null));

export async function packagePlan(options) {
  const output = join(await realpath(dirname(resolve(options.output))), basename(options.output));
  const work = `${output}.build`;
  const log = `${output}.notarization-log.json`;
  for (const path of options.unsigned ? [output, work] : [output, work, log])
    if (!(await absent(path))) throw new Error(`${path} already exists`);
  const release = await readBundle(options.bundle);
  if (output.startsWith(`${release.bundle}${sep}`)) throw new Error('--output must be outside --bundle');
  const content = (path, mode, text) => ({ path, mode, content: text, sha256: sha256(text) });
  const payload = [
    ...release.binaries.map(binary => ({ path: `usr/local/bin/${binary.name}`, mode: 0o755, source: binary.path, sha256: binary.sha256 })),
    content(`${PACKAGE_DIR}/provision-docker-admission-locks.py`, 0o555, await readFile(join(DEPLOY, 'local-linux/provision-docker-admission-locks.py'), 'utf8')),
    content(`Library/LaunchDaemons/${ADMISSION_LABEL}.plist`, 0o644, await readFile(join(DEPLOY, `local-macos/${ADMISSION_LABEL}.plist`), 'utf8')),
    content(`${PACKAGE_DIR}/start-kernel.sh`, 0o555, await renderTemplate('start-kernel.sh', { ROOT: '' })),
    content(`${PACKAGE_DIR}/uninstall.sh`, 0o555, await renderTemplate('uninstall.sh', { ROOT: '' })),
    content(`${PACKAGE_DIR}/bin.sha256`, 0o444, release.binaries.map(binary => `${binary.sha256}  /usr/local/bin/${binary.name}\n`).join('')),
    content(`${PACKAGE_DIR}/slice-build-context.sha256`, 0o444, release.context.files.map(file => `${file.sha256}  /${CONTEXT_DIR}/${file.path}\n`).join('')),
    content(`Library/LaunchAgents/${AGENT_LABEL}.plist`, 0o644, await readFile(join(TEMPLATES, `${AGENT_LABEL}.plist`), 'utf8')),
    ...release.context.files.map(file => ({ path: `${CONTEXT_DIR}/${file.path}`, mode: file.mode, source: file.source })),
  ];
  const postinstall = await renderTemplate('postinstall', { ROOT: '', VERSION: options.version });
  const root = join(work, 'root');
  const scripts = join(work, 'scripts');
  const component = join(work, COMPONENT);
  const distribution = join(work, 'distribution.xml');
  const expanded = join(work, 'expanded');
  const product = options.unsigned ? output : join(work, 'unsigned.pkg');
  // Signed releases accept only Developer ID-signed code of the installer's team.
  const code = options.unsigned ? [] : release.binaries;
  const checks = [
    ...code.map(file => ({ phase: 'code-verify', file, command: [TOOLS.codesign, '--verify', '--strict', '--verbose=2', file.path] })),
    ...code.map(file => ({ phase: 'code-describe', file, command: [TOOLS.codesign, '--display', '--verbose=2', file.path] })),
  ];
  const steps = [
    { phase: 'component', command: [TOOLS.pkgbuild, '--root', root, '--scripts', scripts, '--identifier', IDENTIFIER,
      '--version', options.version, '--install-location', '/', '--ownership', 'recommended',
      '--min-os-version', MIN_MACOS, '--compression', 'latest', component] },
    { phase: 'payload', command: [TOOLS.pkgutil, '--payload-files', component] },
    { phase: 'product', command: [TOOLS.productbuild, '--distribution', distribution, '--package-path', work, product] },
    { phase: 'expand', command: [TOOLS.pkgutil, '--expand', product, expanded] },
  ];
  if (!options.unsigned) steps.push(
    { phase: 'sign', command: [TOOLS.productsign, '--sign', options.identity, '--timestamp', product, output] },
    { phase: 'signature', command: [TOOLS.pkgutil, '--check-signature', output] },
    { phase: 'notarize', command: [TOOLS.xcrun, 'notarytool', 'submit', output, '--keychain-profile', options.keychainProfile,
      '--wait', '--output-format', 'json'] },
    { phase: 'notary-log', command: [TOOLS.xcrun, 'notarytool', 'log', SUBMISSION_ID, '--keychain-profile', options.keychainProfile, log] },
    { phase: 'staple', command: [TOOLS.xcrun, 'stapler', 'staple', output] },
    { phase: 'staple-check', command: [TOOLS.xcrun, 'stapler', 'validate', output] },
    { phase: 'gatekeeper', command: [TOOLS.spctl, '--assess', '--type', 'install', '--verbose=2', output] });
  return { ...options, output, work, log, release, payload, postinstall, distribution: distributionXml({ version: options.version, archs: release.archs }),
    paths: { root, scripts, component, distribution, expanded, product }, checks, steps };
}

const quote = word => /^[A-Za-z0-9_/.,:=@%+-]+$/u.test(word) ? word : `'${word.replaceAll("'", "'\\''")}'`;
const octal = mode => mode.toString(8).padStart(4, '0');

export function formatPlan(plan) {
  const { release } = plan;
  const lines = ['# Dry run: nothing below was executed or written.',
    `# bundle ${release.bundle} (${release.archs.join('+')}); not packaged: ${release.ignored.join(', ') || 'nothing'}`,
    `# postinstall installs the WASM worker with the kernel`];
  for (const entry of plan.payload) if (entry.sha256) lines.push(`# payload ${octal(entry.mode)} /${entry.path}`);
  lines.push(`# payload 0755 /${CONTEXT_DIR}/: the slice build context, ${release.context.files.length} files, `
    + `${release.context.bytes} bytes, listed in /${PACKAGE_DIR}/slice-build-context.sha256`);
  let phase = '';
  for (const step of [...plan.checks, ...plan.steps]) {
    if (step.phase !== phase) lines.push(`# ${(phase = step.phase)}`);
    lines.push(step.command.map(quote).join(' '));
  }
  if (plan.unsigned) lines.push('# unsigned: a test package. No productsign, notarization or staple; Gatekeeper refuses it');
  return `${lines.join('\n')}\n`;
}

async function assemble(plan) {
  const { root, scripts, distribution } = plan.paths;
  await mkdir(plan.work);
  for (const entry of plan.payload) {
    const target = join(root, entry.path);
    await mkdir(dirname(target), { recursive: true });
    // A clone where the volume allows it: no extra disk for the large binaries.
    if (entry.source) await copyFile(entry.source, target, constants.COPYFILE_FICLONE);
    else await writeFile(target, entry.content);
    await chmod(target, entry.mode);
  }
  // Directory modes become the installed ones whatever this process's umask is.
  async function directories(directory, mode) {
    await chmod(directory, mode);
    for (const name of await readdir(directory)) {
      const path = join(directory, name);
      if ((await lstat(path)).isDirectory()) await directories(path, relative(root, path) === `${PACKAGE_DIR}/staging` ? 0o700 : mode);
    }
  }
  await directories(root, 0o755);
  await mkdir(scripts);
  await writeFile(join(scripts, 'postinstall'), plan.postinstall, { mode: 0o755 });
  await writeFile(distribution, plan.distribution);
}

// pkgbuild stores extended attributes (here com.apple.provenance, which macOS
// adds to files a provenance-tracked app writes) as AppleDouble ._ entries; the
// Installer restores them as attributes, not files. Anything else is a mismatch.
function checkPayload(listing, plan) {
  const expected = new Set(['.']);
  for (const entry of plan.payload) {
    const parts = entry.path.split('/');
    for (let depth = 1; depth <= parts.length; depth += 1) expected.add(`./${parts.slice(0, depth).join('/')}`);
  }
  const listed = new Set(listing.split('\n').filter(Boolean));
  let appleDouble = 0;
  for (const path of listed) {
    if (expected.has(path)) continue;
    const companion = path.replace(/\/\._([^/]+)$/u, '/$1');
    if (companion !== path && expected.has(companion)) { appleDouble += 1; continue; }
    throw new Error(`the component payload has an unexpected entry ${path}`);
  }
  const missing = [...expected].filter(path => !listed.has(path));
  if (missing.length) throw new Error(`the component payload lacks ${missing.slice(0, 3).join(', ')}`);
  return appleDouble;
}

async function checkExpanded(plan) {
  const { expanded } = plan.paths;
  // productbuild rewrites the XML declaration and the component's pkg-ref (adding its sizes); every other line must ship.
  const shipped = new Set((await readFile(join(expanded, 'Distribution'), 'utf8')).split('\n').map(line => line.trim()));
  for (const line of plan.distribution.split('\n').map(text => text.trim()))
    if (line && !line.startsWith('<?xml') && !line.includes(`version="${plan.version}" onConclusion=`) && !shipped.has(line))
      throw new Error(`the product's Distribution lacks ${line}`);
  const info = await readFile(join(expanded, COMPONENT, 'PackageInfo'), 'utf8');
  for (const needle of [`identifier="${IDENTIFIER}"`, `version="${plan.version}"`, 'install-location="/"', 'auth="root"',
    `minimumSystemVersion="${MIN_MACOS}"`, '<postinstall file="./postinstall"'])
    if (!info.includes(needle)) throw new Error(`the component PackageInfo lacks ${needle}`);
  if (await readFile(join(expanded, COMPONENT, 'Scripts', 'postinstall'), 'utf8') !== plan.postinstall)
    throw new Error('the product\'s postinstall is not the one rendered with the release binaries');
  const scripts = (await readdir(join(expanded, COMPONENT, 'Scripts'))).filter(name => !name.startsWith('._'));
  if (scripts.join() !== 'postinstall') throw new Error(`the product has unexpected scripts: ${scripts.join(', ')}`);
}

function describeCode(result, step, teamId) {
  const text = `${result.stderr}\n${result.stdout}`;
  const lines = text.split('\n').map(line => line.trim());
  if (!lines.some(line => line.startsWith('Authority=Developer ID Application: ') && line.endsWith(` (${teamId})`)))
    throw new Error(`${step.file.path} is not Developer ID Application-signed by team ${teamId}; sign the bundle with sign-macos-release.mjs`);
  if (!lines.includes(`TeamIdentifier=${teamId}`)) throw new Error(`${step.file.path} has the wrong team identifier`);
  if (!lines.some(line => line.startsWith('Timestamp='))) throw new Error(`${step.file.path} has no secure timestamp`);
  if (!/flags=0x[0-9a-f]+\([^)]*\bruntime\b[^)]*\)/u.test(text)) throw new Error(`${step.file.path} lacks the hardened runtime`);
}

function notarization(result, profile) {
  let body;
  try { body = JSON.parse(result.stdout); } catch { throw new Error('notarytool returned no JSON result'); }
  if (typeof body?.id !== 'string' || !/^[A-Za-z0-9-]{1,64}$/u.test(body.id)) throw new Error('notarytool returned no submission id');
  if (body.status !== 'Accepted')
    throw new Error(`notarization ${body.status}; inspect it with: xcrun notarytool log ${body.id} --keychain-profile ${profile}`);
  return body.id;
}

function runCommand(command) {
  const notarize = command[1] === 'notarytool' && command[2] === 'submit';
  const result = spawnSync(command[0], command.slice(1), { encoding: 'utf8', maxBuffer: 16 * 1024 * 1024,
    timeout: notarize ? 2 * 60 * 60 * 1000 : 20 * 60 * 1000, stdio: ['ignore', 'pipe', 'pipe'] });
  return { status: result.status, stdout: result.stdout ?? '', stderr: result.stderr ?? result.error?.message ?? '' };
}

function checked(run, step) {
  const result = run(step.command);
  if (result.status !== 0) {
    const detail = `${result.stderr}${result.stdout}`.trim().slice(-2000);
    throw new Error(`${step.phase} failed${step.file ? ` for ${step.file.path}` : ''}${detail ? `: ${detail}` : ''}`);
  }
  return result;
}

export async function buildMacosPkg(options, { run = runCommand, platform = process.platform } = {}) {
  const plan = await packagePlan(options);
  if (options.dryRun) return { plan };
  if (platform !== 'darwin') throw new Error('the macOS package builds on macOS');
  for (const step of plan.checks) {
    const result = checked(run, step);
    if (step.phase === 'code-describe') describeCode(result, step, options.teamId);
  }
  let submissionId;
  let log = plan.unsigned ? null : plan.log;
  let appleDouble = 0;
  try {
    await assemble(plan);
    for (const step of plan.steps) {
      if (step.phase === 'notary-log') {
        // The log is evidence; an accepted submission stands without it.
        if (run(step.command.map(word => word === SUBMISSION_ID ? submissionId : word)).status !== 0) log = 'unavailable';
        continue;
      }
      const result = checked(run, step);
      if (step.phase === 'payload') appleDouble = checkPayload(result.stdout, plan);
      else if (step.phase === 'product') await rm(plan.paths.component);
      else if (step.phase === 'expand') { await checkExpanded(plan); await rm(plan.paths.expanded, { recursive: true }); }
      else if (step.phase === 'signature') {
        if (!result.stdout.includes(`Developer ID Installer: `) || !result.stdout.includes(`(${options.teamId})`)
          || !/Signed with a trusted timestamp/u.test(result.stdout))
          throw new Error('the product signature is not a timestamped Developer ID Installer signature of the named team');
      } else if (step.phase === 'notarize') submissionId = notarization(result, options.keychainProfile);
      else if (step.phase === 'gatekeeper' && !/source=Notarized Developer ID/u.test(`${result.stderr}\n${result.stdout}`))
        throw new Error('Gatekeeper does not accept the package as notarized Developer ID');
    }
    const receipt = {
      schema: 'chariox.macos-pkg.v1', identifier: IDENTIFIER, version: plan.version, output: plan.output,
      sha256: await hashFile(plan.output), bytes: (await lstat(plan.output)).size,
      architectures: plan.release.archs, minimumMacos: MIN_MACOS,
      sliceBuildContext: { path: `/${CONTEXT_DIR}`, files: plan.release.context.files.length, bytes: plan.release.context.bytes },
      payload: plan.payload.filter(entry => entry.sha256).map(entry => ({ path: `/${entry.path}`, mode: octal(entry.mode), sha256: entry.sha256 })),
      appleDoubleEntries: appleDouble, notPackaged: plan.release.ignored,
      signing: plan.unsigned ? 'unsigned test package: Gatekeeper refuses it'
        : { identity: options.identity, teamId: options.teamId, keychainProfile: options.keychainProfile,
          notarization: { submissionId, status: 'Accepted', log }, stapled: true, gatekeeper: 'Notarized Developer ID' },
    };
    await rm(plan.work, { recursive: true, force: true });
    return { plan, receipt };
  } catch (error) {
    // The plan proved these paths absent, so a failed build removes them and a retry can reuse them.
    await Promise.all([plan.work, plan.output, ...(plan.unsigned ? [] : [plan.log])].map(path => rm(path, { recursive: true, force: true })));
    if (submissionId) error.message += `; accepted notarization ${submissionId}: `
      + `xcrun notarytool log ${submissionId} --keychain-profile ${options.keychainProfile}`;
    throw error;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const options = parseArguments(process.argv.slice(2));
    const { plan, receipt } = await buildMacosPkg(options);
    process.stdout.write(options.dryRun ? formatPlan(plan) : `${JSON.stringify(receipt, null, 2)}\n`);
  } catch (error) {
    process.stderr.write(`macos_pkg_build_failed: ${error.message}\n`);
    process.exitCode = error instanceof UsageError ? 2 : 1;
  }
}
