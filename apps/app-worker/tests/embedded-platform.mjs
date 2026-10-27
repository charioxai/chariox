// Hosted fixture observation/provisioning helpers. No App-facing surface.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { open, readlink, realpath, stat } from 'node:fs/promises';
import { join } from 'node:path';

const systemNames = new Set(['libstdc++.so.6', 'libgcc_s.so.1', 'libm.so.6', 'libc.so.6',
  'libpthread.so.0', 'libdl.so.2', 'librt.so.1', 'libatomic.so.1', 'ld-linux-x86-64.so.2']);

export async function boundedText(path) {
  const file = await open(path, 'r');
  try {
    const buffer = Buffer.alloc(65537); let used = 0;
    while (used < buffer.length) {
      const result = await file.read(buffer, used, buffer.length - used, used);
      if (!result.bytesRead) break;
      used += result.bytesRead;
    }
    assert.ok(used <= 65536);
    return buffer.subarray(0, used).toString('utf8');
  } finally { await file.close(); }
}

export async function systemLibraryMounts(bundle, launcher) {
  const queue = [launcher, join(bundle, 'libnode.so.137'), join(bundle, 'libchariox-app-runtime.so')];
  const mounts = new Map();
  while (queue.length) {
    const file = queue.shift();
    const text = execFileSync('/usr/bin/readelf', ['-d', file], { encoding: 'utf8', timeout: 5000, maxBuffer: 65536,
      env: { PATH: '/usr/bin:/bin', LANG: 'C' }, stdio: ['ignore', 'pipe', 'pipe'] });
    for (const match of text.matchAll(/\((?:RUNPATH|RPATH)\).*\[([^\]]*)\]/g)) assert.equal(match[1], '$ORIGIN');
    for (const match of text.matchAll(/\(NEEDED\).*\[([^\]]+)\]/g)) {
      const name = match[1];
      if (name === 'libnode.so.137') continue;
      assert.ok(systemNames.has(name), 'undeclared system library');
      if (mounts.has(name)) continue;
      const source = await realpath(`/lib/x86_64-linux-gnu/${name}`);
      assert.ok(source.startsWith('/usr/lib/x86_64-linux-gnu/') || source.startsWith('/lib/x86_64-linux-gnu/'));
      const metadata = await stat(source);
      assert.ok(metadata.isFile() && metadata.uid === 0 && !(metadata.mode & 0o022));
      mounts.set(name, { source, target: `/lib/x86_64-linux-gnu/${name}` });
      queue.push(source);
    }
  }
  const interpreter = await realpath('/lib64/ld-linux-x86-64.so.2');
  assert.ok(interpreter.startsWith('/usr/lib/x86_64-linux-gnu/') || interpreter.startsWith('/lib/x86_64-linux-gnu/'));
  const interpreterMetadata = await stat(interpreter);
  assert.ok(interpreterMetadata.isFile() && interpreterMetadata.uid === 0 && !(interpreterMetadata.mode & 0o022));
  return [...mounts.values(), { source: interpreter, target: '/lib64/ld-linux-x86-64.so.2' }];
}

export async function namespaces(pid = 'self') {
  return Object.fromEntries(await Promise.all(['mnt', 'user', 'pid', 'net', 'ipc', 'uts', 'cgroup']
    .map(async name => [name, await readlink(`/proc/${pid}/ns/${name}`)])));
}

export async function inspectWorker(pid, { baseline, unit, uid, gid, launcher, roots }) {
  assert.ok(Number.isSafeInteger(pid) && pid > 0);
  const status = await boundedText(`/proc/${pid}/status`);
  for (const field of ['CapEff', 'CapPrm', 'CapInh']) assert.match(status, new RegExp(`^${field}:\\s+0000000000000000$`, 'm'));
  assert.match(status, /^NoNewPrivs:\s+1$/m); assert.match(status, /^Seccomp:\s+2$/m);
  const values = field => status.match(new RegExp(`^${field}:\\s+(.+)$`, 'm'))[1].trim().split(/\s+/).map(Number);
  assert.deepEqual(values('Uid'), [uid, uid, uid, uid]); assert.deepEqual(values('Gid'), [gid, gid, gid, gid]);
  assert.match(status, /^Groups:[ \t]*$/m);
  const observed = await namespaces(pid);
  for (const [name, value] of Object.entries(observed)) assert.notEqual(value, baseline[name]);
  assert.equal((await boundedText(`/proc/${pid}/cgroup`)).trim(), `0::/system.slice/${unit}.service`);
  const mountinfo = await boundedText(`/proc/${pid}/mountinfo`);
  const mounts = {};
  for (const root of ['/', ...Object.values(roots)]) {
    const line = mountinfo.split('\n').find(line => line.split(' ')[4] === root);
    assert.ok(line, 'missing admitted mount');
    const flags = line.split(' ')[5].split(','); mounts[root] = flags;
    if (root === '/' || root === roots.package || root === roots.runtime) assert.ok(flags.includes('ro'));
    if (root !== '/') {
      assert.ok(flags.includes('nodev') && flags.includes('nosuid'));
      if (root !== roots.runtime) assert.ok(flags.includes('noexec'));
    }
  }
  await assert.rejects(stat(`/proc/${pid}/root/proc`), { code: 'ENOENT' });
  await assert.rejects(stat(`/proc/${pid}/root/etc/passwd`), { code: 'ENOENT' });
  const executable = await stat(`/proc/${pid}/exe`); const expected = await stat(launcher);
  assert.equal(executable.dev, expected.dev); assert.equal(executable.ino, expected.ino);
  return { namespaces: observed, mounts, uid, gid, supplementaryGroups: [], capabilities: 'none', noNewPrivileges: true, seccomp: true };
}
