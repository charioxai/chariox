// MP-08 / MP-10 / MP-11: process-owned file locks on the Linux builder and macOS coordinator.
import { open } from 'node:fs/promises';
import { constants } from 'node:fs';
import { spawn } from 'node:child_process';

// Darwin's O_EXLOCK is not exported by Node. libuv passes numeric flags to open(2),
// with close-on-exec, so provider children cannot retain the driver's lock.
// https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/fcntl.h
const DARWIN_O_EXLOCK = 0x20;
export async function tryProcessFileLock(path, { platform = process.platform, openFile = open, spawnProcess = spawn } = {}) {
  if (platform === 'darwin') {
    let file;
    try { file = await openFile(path, constants.O_RDWR | constants.O_CREAT | constants.O_NONBLOCK | DARWIN_O_EXLOCK, 0o600); }
    catch (error) { if (['EAGAIN', 'EWOULDBLOCK'].includes(error.code)) return undefined; throw error; }
    // A file descriptor alone does not keep Node alive; retain the fd until release or driver death.
    const keepAlive = setInterval(() => file.fd, 1 << 30);
    return { unlock: async () => { clearInterval(keepAlive); await file.close(); } };
  }
  const child = spawnProcess('flock', ['--exclusive', '--nonblock', path,
    'sh', '-c', 'printf "locked\\n"; cat >/dev/null'], { stdio: ['pipe', 'pipe', 'pipe'] });
  child.stdin.on('error', () => {});
  const exited = new Promise((resolve, reject) => { child.once('error', reject); child.once('close', resolve); });
  const acquired = await Promise.race([
    new Promise(resolve => child.stdout.once('data', () => resolve(true))),
    exited.then(code => { if (code === 1) return false; throw new Error(`lane file lock failed (${code}); inspect the Linux flock installation`); }),
  ]);
  return acquired ? { unlock: async () => { child.stdin.end(); await exited; } } : undefined;
}
