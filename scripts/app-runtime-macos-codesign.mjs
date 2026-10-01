// macOS code signatures on App runtime release files, as the Chariox release
// signer checks them. Code signing rewrites only a thin Mach-O file's embedded
// signature, so a signed file must equal its builder-attested unsigned bytes
// everywhere outside that signature. codesign then checks the signature, the
// hardened runtime and the exact entitlements; production releases also need
// the named Developer ID team and a notarized worker. The Chariox runtime
// inventory is signed over these checked bytes. Nothing here reads a credential.
import { spawnSync } from 'node:child_process';
import { chmod, cp } from 'node:fs/promises';
import { basename, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const MH_MAGIC_64 = 0xfeedfacf;
const LC_SEGMENT_64 = 0x19;
const LC_CODE_SIGNATURE = 0x1d;
const HEADER = 32;
export const JIT_ENTITLEMENT = 'com.apple.security.cs.allow-jit';
// The same file and the same single entitlement as sign-macos-release.mjs.
export const WORKER_ENTITLEMENTS = join(dirname(fileURLToPath(import.meta.url)), 'macos-app-worker.entitlements');
const IDENTITY = /^Developer ID Application: [^\r\n]+ \(([A-Z0-9]{10})\)$/u;
const CODESIGN = '/usr/bin/codesign';
const SPCTL = '/usr/sbin/spctl';

function layout(bytes, path) {
  const fail = reason => { throw new Error(`${path} ${reason}`); };
  if (bytes.length < HEADER || bytes.readUInt32LE(0) !== MH_MAGIC_64) fail('is not a thin 64-bit Mach-O file');
  const ncmds = bytes.readUInt32LE(16);
  const sizeofcmds = bytes.readUInt32LE(20);
  const end = HEADER + sizeofcmds;
  if (end > bytes.length) fail('has truncated load commands');
  let offset = HEADER;
  let signature = null;
  let linkedit = null;
  for (let index = 0; index < ncmds; index += 1) {
    if (offset + 8 > end) fail('has truncated load commands');
    const command = bytes.readUInt32LE(offset);
    const size = bytes.readUInt32LE(offset + 4);
    if (size < 8 || size % 8 || offset + size > end) fail('has a malformed load command');
    if (command === LC_CODE_SIGNATURE) {
      if (signature || size !== 16 || index !== ncmds - 1) fail('must end its load commands with one code signature');
      signature = { offset, start: bytes.readUInt32LE(offset + 8), size: bytes.readUInt32LE(offset + 12) };
    } else if (command === LC_SEGMENT_64 && size >= 72
      && bytes.toString('latin1', offset + 8, offset + 24).replace(/\0+$/u, '') === '__LINKEDIT') {
      if (linkedit) fail('has two __LINKEDIT segments');
      linkedit = { offset, start: Number(bytes.readBigUInt64LE(offset + 40)), size: Number(bytes.readBigUInt64LE(offset + 48)) };
    }
    offset += size;
  }
  if (offset !== end || !linkedit || linkedit.start + linkedit.size !== bytes.length) fail('must end with its __LINKEDIT segment');
  if (signature && (signature.start % 16 || signature.start < linkedit.start || signature.start + signature.size !== bytes.length))
    fail('must end with its code signature');
  return { ncmds, sizeofcmds, signature, linkedit, end: signature ? signature.start : bytes.length };
}

// The bytes a signature covers, with the fields that only describe the signature
// cleared: its load command and the header counts that include it, and the
// __LINKEDIT sizes that grow to hold it. Every other byte must stay identical.
function covered(bytes, shape) {
  const copy = Buffer.from(bytes.subarray(0, shape.end));
  if (shape.signature) {
    copy.writeUInt32LE(shape.ncmds - 1, 16);
    copy.writeUInt32LE(shape.sizeofcmds - 16, 20);
    copy.fill(0, shape.signature.offset, shape.signature.offset + 16);
  }
  copy.fill(0, shape.linkedit.offset + 32, shape.linkedit.offset + 40);
  copy.fill(0, shape.linkedit.offset + 48, shape.linkedit.offset + 56);
  return copy;
}

export function requireSignatureOnlyChange(unsigned, signed, path) {
  const before = covered(unsigned, layout(unsigned, path));
  const shape = layout(signed, path);
  if (!shape.signature) throw new Error(`${path} is not code signed`);
  const after = covered(signed, shape);
  // Signing a file without a signature pads it to the 16-byte signature alignment.
  const padding = after.subarray(before.length);
  if (after.length < before.length || padding.length >= 16 || padding.some(byte => byte !== 0)
    || !before.equals(after.subarray(0, before.length)))
    throw new Error(`${path} differs from its attested unsigned bytes outside the code signature`);
}

// A signature adds page hashes (at most 52 bytes per 4 KiB page across both
// code directories), requirements, entitlements and a certificate chain.
export const signedLimit = size => size + Math.ceil(size / 64) + 1024 * 1024;

function runCommand(command) {
  const result = spawnSync(command[0], command.slice(1), { encoding: 'utf8', maxBuffer: 16 * 1024 * 1024,
    timeout: 10 * 60 * 1000, stdio: ['ignore', 'pipe', 'pipe'] });
  return { status: result.status, stdout: result.stdout ?? '', stderr: result.stderr ?? result.error?.message ?? '' };
}

function checked(run, command, what) {
  const result = run(command);
  if (result.status !== 0) {
    const detail = `${result.stderr}${result.stdout}`.trim().slice(-2000);
    throw new Error(`${what} failed${detail ? `: ${detail}` : ''}`);
  }
  return result;
}

// Apple's Developer ID Application requirement, pinned to one team.
export const developerIdRequirement = team => '=anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists'
  + ` and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = "${team}"`;

export function teamOf(identity) {
  const match = IDENTITY.exec(identity ?? '');
  if (!match) throw new Error('codesign identity must be "Developer ID Application: <Name> (<TEAMID>)"');
  return match[1];
}

// Checks one installed-to-be file. `identity` is required for production; the
// developer path accepts any valid signature with the production shape.
export function verifyCodeSignature(file, { jit, executable, identity, run = runCommand }) {
  const name = basename(file);
  checked(run, [CODESIGN, '--verify', '--strict', '--verbose=2', file], `signature check for ${name}`);
  const shown = checked(run, [CODESIGN, '--display', '--verbose=2', file], `signature display for ${name}`);
  const text = `${shown.stderr}\n${shown.stdout}`;
  if (!/flags=0x[0-9a-f]+\([^)]*\bruntime\b[^)]*\)/u.test(text)) throw new Error(`${name} lacks the hardened runtime`);
  const granted = checked(run, [CODESIGN, '--display', '--entitlements', '-', '--xml', file], `entitlements display for ${name}`);
  const keys = [...granted.stdout.matchAll(/<key>([^<]+)<\/key>/gu)].map(match => match[1]);
  if (jit ? keys.length !== 1 || keys[0] !== JIT_ENTITLEMENT : keys.length)
    throw new Error(`${name} has unexpected entitlements: ${keys.join(', ') || 'none'}`);
  if (identity === undefined) return;
  const team = teamOf(identity);
  const lines = new Set(text.split('\n').map(line => line.trim()));
  if (!lines.has(`Authority=${identity}`) || !lines.has(`TeamIdentifier=${team}`))
    throw new Error(`${name} is not signed by the named Developer ID identity`);
  if (![...lines].some(line => line.startsWith('Timestamp='))) throw new Error(`${name} has no secure timestamp`);
  checked(run, [CODESIGN, '--verify', '--strict', '-R', developerIdRequirement(team), file], `Developer ID check for ${name}`);
  if (!executable) return;
  const assessed = checked(run, [SPCTL, '--assess', '--type', 'install', '--verbose=2', file], `Gatekeeper assessment for ${name}`);
  if (!/source=Notarized Developer ID/u.test(`${assessed.stderr}\n${assessed.stdout}`))
    throw new Error(`${name} is not accepted as notarized Developer ID code`);
}

// Developer path only: codesign a copy of an assembled release input the way
// sign-macos-release.mjs does, without notarization. `identity` may be "-".
export async function codesignCopy({ input, output, code, identity }, { run = runCommand } = {}) {
  await cp(input, output, { recursive: true, errorOnExist: true, force: false });
  // Libraries before the executable that loads them.
  const ordered = [...code].sort((a, b) => Number(a.executable) - Number(b.executable) || (a.path < b.path ? -1 : 1));
  for (const file of ordered) {
    const path = join(output, file.path);
    await chmod(path, file.executable ? 0o755 : 0o644);
    checked(run, [CODESIGN, '--force', '--sign', identity, '--timestamp', '--options', 'runtime',
      ...(file.jit ? ['--entitlements', WORKER_ENTITLEMENTS] : []), path], `codesign of ${file.path}`);
  }
}
