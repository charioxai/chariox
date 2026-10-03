// Test-only App. Executed only by the confined hosted embedded-runtime fixture.
import * as fs from 'node:fs';
import { promises as files } from 'node:fs';
import { join } from 'node:path';
import { createConnection } from 'node:net';
import { createSocket } from 'node:dgram';
import { spawnSync } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { createRequire } from 'node:module';
import { esmValue } from './value.mjs';

fs.writeFileSync(join(process.env.CHARIOX_APP_DATA, 'app-imported'), 'after confinement');

function denied(operation, codes = ['ERR_ACCESS_DENIED', 'EPERM', 'EACCES', 'EROFS', 'ENOENT']) {
  try { operation(); return false; } catch (error) { return codes.includes(error.code); }
}

async function networkDenied(udp) {
  return new Promise(resolve => {
    let socket;
    let done = false;
    const finish = value => {
      if (done) return;
      done = true; clearTimeout(timer);
      try { udp ? socket?.close() : socket?.destroy(); } catch { /* unbound UDP socket */ }
      resolve(value);
    };
    const timer = setTimeout(() => finish(false), 1000);
    try {
      socket = udp ? createSocket('udp4') : createConnection({ host: '127.0.0.1', port: 9 });
      socket.once('error', error => finish(['EPERM', 'EACCES', 'ERR_ACCESS_DENIED'].includes(error.code)));
      socket.once(udp ? 'listening' : 'connect', () => finish(false));
      if (udp) socket.bind(0, '127.0.0.1');
    } catch (error) { finish(['EPERM', 'EACCES', 'ERR_ACCESS_DENIED'].includes(error.code)); }
  });
}

export default function register(chariox) {
  chariox.tools.register('probe', async () => {
    const data = chariox.paths.data;
    const source = join(data, 'original'); const copied = join(data, 'copied');
    await files.writeFile(source, 'standard private I/O');
    await files.copyFile(source, copied);
    let watcher;
    let timer;
    const watched = new Promise((resolve, reject) => {
      watcher = fs.watch(data, (_event, name) => { if (name?.toString() === 'watch-marker') resolve(true); });
      watcher.on('error', reject);
      timer = setTimeout(() => reject(new Error('private watch deadline')), 1000);
    });
    try {
      await files.writeFile(join(data, 'watch-marker'), 'watch');
      await watched;
    } finally { clearTimeout(timer); watcher?.close(); }
    await files.writeFile(join(chariox.paths.temporary, 'temporary'), 'private');
    const cjs = createRequire(import.meta.url)('./value.cjs');
    return {
      nodeVersion: process.versions.node, moduleAbi: process.versions.modules,
      esmAndCjs: esmValue === 'esm' && cjs === 'cjs', sdkFrozen: Object.isFrozen(chariox),
      privateIo: await files.readFile(copied, 'utf8') === 'standard private I/O', privateWatch: true,
      crypto: randomBytes(16).length === 16 && createHash('sha256').update('fixture').digest('hex').length === 64,
      environmentFiltered: !process.env.GITHUB_TOKEN && !process.env.PATH && !process.env.NODE_OPTIONS && !process.env.CX_TEST_SECRET,
      ambientFdDenied: denied(() => fs.fstatSync(31), ['EBADF', 'ERR_ACCESS_DENIED']),
      hostReadDenied: denied(() => fs.readFileSync('/etc/passwd')),
      escapedReadDenied: denied(() => fs.readFileSync(join(chariox.paths.package, 'escape'))),
      packageWriteDenied: denied(() => fs.writeFileSync(join(chariox.paths.package, 'forbidden'), 'x')),
      childDenied: denied(() => {
        const result = spawnSync('/usr/bin/false');
        if (result.error) throw result.error;
      }, ['ERR_ACCESS_DENIED', 'EPERM', 'EACCES']),
      addonDenied: denied(() => process.dlopen({ exports: {} }, join(chariox.paths.package, 'addon.node')),
        ['ERR_ACCESS_DENIED', 'ERR_DLOPEN_DISABLED', 'EPERM', 'EACCES']),
      tcpDenied: await networkDenied(false), udpDenied: await networkDenied(true),
    };
  });
  chariox.lifecycle.on('shutdown', () => 'shutdown response drained');
}
