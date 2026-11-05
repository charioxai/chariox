// Bounded filesystem operations for an unsigned build artifact. No execution.
import { createHash } from 'node:crypto';
import { constants } from 'node:fs';
import { lstat, mkdir, open, opendir, realpath } from 'node:fs/promises';
import { dirname, isAbsolute, join, resolve } from 'node:path';

export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
export const stableJson = value => JSON.stringify(value, (_key, item) => {
  if (item && typeof item === 'object' && !Array.isArray(item))
    return Object.fromEntries(Object.keys(item).sort().map(key => [key, item[key]]));
  return item;
});

export function relativeFile(path) {
  if (typeof path !== 'string' || path.length > 256 || path.startsWith('/')
    || /[\\\x00-\x1f\x7f]/u.test(path) || path.split('/').some(part => !part || part === '.' || part === '..'))
    throw new Error('invalid bundle inventory path');
  return path;
}

// This is a trusted build tool, not the installer's descriptor-anchored boundary.
// The builder must exclusively control inputs/output for the whole operation.
// Reject other-user writable parents; a root-owned sticky temporary parent is
// safe for a builder-owned child, but does not protect against the same UID.
export async function builderDirectory(path) {
  if (!isAbsolute(path) || resolve(path) !== path || await realpath(path) !== path)
    throw new Error('builder directory must be canonical');
  for (let current = path; ; current = dirname(current)) {
    const metadata = await lstat(current);
    if (!metadata.isDirectory() || ![0, process.getuid()].includes(metadata.uid)
      || metadata.mode & 0o022 && !(metadata.uid === 0 && metadata.mode & 0o1000))
      throw new Error('builder directory chain must be trusted and not writable by other users');
    if (current === dirname(current)) break;
  }
}

export async function openRegular(root, relative, maximum) {
  const path = join(root, relativeFile(relative));
  await builderDirectory(dirname(path));
  if (await realpath(path) !== path) throw new Error('bundle inputs must not contain symlinks');
  const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const metadata = await file.stat();
    if (!metadata.isFile() || metadata.nlink !== 1 || metadata.size > maximum
      || ![0, process.getuid()].includes(metadata.uid) || metadata.mode & 0o022)
      throw new Error('bundle input must be a bounded regular file with one link');
    return { file, metadata };
  } catch (error) { await file.close(); throw error; }
}

export async function readSmall(root, relative, maximum) {
  const { file } = await openRegular(root, relative, maximum);
  try {
    const bytes = Buffer.alloc(maximum + 1);
    let size = 0;
    for (;;) {
      const read = await file.read(bytes, size, bytes.length - size, size);
      if (!read.bytesRead) break;
      size += read.bytesRead;
      if (size > maximum) throw new Error('bundle input exceeded its byte limit');
    }
    return bytes.subarray(0, size);
  } finally { await file.close(); }
}

export async function inventory(root, maximumFiles) {
  await builderDirectory(root);
  const files = [];
  let entries = 0;
  async function walk(directory, prefix = '') {
    for await (const entry of await opendir(directory, { bufferSize: 16 })) {
      if (++entries > maximumFiles * 3) throw new Error('bundle inventory exceeded entry limit');
      const relative = relativeFile(prefix + entry.name);
      const metadata = await lstat(join(root, relative));
      if (metadata.isDirectory()) {
        if (![0, process.getuid()].includes(metadata.uid) || metadata.mode & 0o022)
          throw new Error('bundle subdirectory must be builder-controlled');
        const before = files.length;
        await walk(join(root, relative), `${relative}/`);
        if (files.length === before) throw new Error('bundle inventory contains an empty directory');
      }
      else if (metadata.isFile() && metadata.nlink === 1) files.push(relative);
      else throw new Error('bundle inventory contains a link or special file');
      if (files.length > maximumFiles) throw new Error('bundle inventory exceeded file limit');
    }
  }
  await walk(root);
  return files.sort();
}

export async function digestAndCopy(root, relative, outputRoot, outputPath, maximum, expected) {
  const { file, metadata } = await openRegular(root, relative, maximum);
  let destination;
  try {
    if (outputRoot) {
      const path = join(outputRoot, relativeFile(outputPath));
      await mkdir(dirname(path), { recursive: true, mode: 0o700 });
      destination = await open(path, 'wx', 0o600);
    }
    const hash = createHash('sha256');
    const buffer = Buffer.alloc(65536);
    let size = 0;
    for (;;) {
      const { bytesRead } = await file.read(buffer, 0, buffer.length, size);
      if (!bytesRead) break;
      size += bytesRead;
      if (size > maximum) throw new Error('bundle input exceeded its byte limit');
      const bytes = buffer.subarray(0, bytesRead);
      hash.update(bytes);
      if (destination) await destination.writeFile(bytes);
    }
    const digest = hash.digest('hex');
    if (size !== metadata.size || expected && (size !== expected.size || digest !== expected.sha256))
      throw new Error('bundle input digest or size mismatch');
    if (destination) { await destination.chmod(0o444); await destination.sync(); }
    // Intended publication mode; verification checks bytes, not immutability.
    // Artifact transports can change modes and the builder remains the owner.
    return { path: outputPath ?? relative, size, mode: '0444', sha256: digest };
  } finally { await destination?.close(); await file.close(); }
}

export async function outputDirectory(path, repository, input) {
  if (!isAbsolute(path) || resolve(path) !== path || path === '/') throw new Error('bundle output must be an absolute new directory');
  const parent = dirname(path);
  await builderDirectory(parent);
  const metadata = await lstat(parent);
  if (!metadata.isDirectory() || metadata.uid !== process.getuid() || metadata.mode & 0o022)
    throw new Error('bundle output parent must be owned and not writable by other users');
  if ([repository, input].some(root => path === root || path.startsWith(`${root}/`) || root.startsWith(`${path}/`)))
    throw new Error('bundle output must be separate from source and native inputs');
  for (let current = parent; ; current = dirname(current)) {
    if (await lstat(join(current, '.git')).catch(error => error.code === 'ENOENT' ? null : Promise.reject(error)))
      throw new Error('bundle output cannot be inside a repository');
    if (current === dirname(current)) break;
  }
  await mkdir(path, { mode: 0o700 });
  return lstat(path);
}
