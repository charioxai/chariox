// MP-08 / MP-09 / MP-10 / MP-11: real binary exec with a deterministic learned peak.
import { createHash } from 'node:crypto'
import { mkdir, realpath, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { Fixture } from './fixture.mjs'

const script = 'import sys,hashlib; open(sys.argv[1],"ab").write(b"x"); data=sys.stdin.buffer.read(); sys.stdout.buffer.write(data); sys.stderr.write(hashlib.sha256(data).hexdigest()); sys.exit(23)'
export const marker = (home, name, host = 'A') => path.join(home, host, 'work', name)
export const command = (home, name, host = 'A') => ['/usr/bin/python3', path.join(home, host, 'work', 'binary-stream.py'), name]

// The kernel's peak key (normalized line plus folder) and the exact line a peak decides alone for.
function key(argv, cwd) {
  const hash = createHash('sha256'), exact = createHash('sha256')
  for (const argument of argv) {
    const normalized = argument.replace(/[a-fA-F0-9]+/g, run => run.length >= 6 || /^\d+$/.test(run) ? '#' : run)
    hash.update(normalized); hash.update(Buffer.from([0]))
    exact.update(argument); exact.update(Buffer.from([0]))
  }
  hash.update(cwd)
  return {key: hash.digest('hex').slice(0, 32), exact: exact.digest().readBigUInt64LE(0)}
}

export class AdmissionFixture extends Fixture {
  async launch(name, executable, env, argv = []) {
    if (name === 'kernel-A' || name === 'kernel-B') {
      const host = name.at(-1)
      const root = path.join(env.CHARIOX_HOME, 'state')
      await mkdir(root, {recursive: true, mode: 0o700})
      const cwd = await realpath(path.join(this.home, host, 'work'))
      await writeFile(path.join(cwd, 'binary-stream.py'), script, {mode: 0o600})
      // This is the ordinary learned-peak input to admission, not a gate override. The peak
      // decides alone only for the exact line, so each entry carries it. Above the pool a
      // matching peak ends the launch (it can never fit); `peakBytes` below the pool holds it
      // while the room is occupied.
      const names = ['cancel', 'start', 'revoke', 'peer-start', 'peer-cancel', 'peer-revoke', 'peer-target-revoke', 'ui-start-standard', 'ui-cancel-standard', 'ui-start-retina', 'ui-cancel-retina', 'ui-owner-standard', 'ui-owner-retina', 'ui-place-standard', 'ui-place-retina', 'ui-stay-standard', 'ui-stay-retina']
      // u64 values do not survive JSON.stringify of a Number: write the text.
      const peaks = `{${names.map(name => { const seen = key(command(this.home, name, host), cwd); return `"${seen.key}":{"memory":${this.peakBytes ?? 2 ** 52},"writes":0,"exact":${seen.exact}}` }).join(',')}}`
      await writeFile(path.join(root, 'admission-peaks.json'), peaks, {mode: 0o600})
    }
    return super.launch(name, executable, env, argv)
  }
}
