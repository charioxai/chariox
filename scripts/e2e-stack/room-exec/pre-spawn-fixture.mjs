// MP-08 / MP-09 / MP-10 / MP-11: real binary exec with a deterministic learned peak.
import { createHash } from 'node:crypto'
import { mkdir, realpath, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { Fixture } from './fixture.mjs'

const script = 'import sys,hashlib; open(sys.argv[1],"ab").write(b"x"); data=sys.stdin.buffer.read(); sys.stdout.buffer.write(data); sys.stderr.write(hashlib.sha256(data).hexdigest()); sys.exit(23)'
export const marker = (home, name, host = 'A') => path.join(home, host, 'work', name)
export const command = (home, name, host = 'A') => ['/usr/bin/python3', path.join(home, host, 'work', 'binary-stream.py'), name]

function key(argv, cwd) {
  const hash = createHash('sha256')
  for (const argument of argv) {
    const normalized = argument.replace(/[a-fA-F0-9]+/g, run => run.length >= 6 || /^\d+$/.test(run) ? '#' : run)
    hash.update(normalized); hash.update(Buffer.from([0]))
  }
  hash.update(cwd)
  return hash.digest('hex').slice(0, 32)
}

export class AdmissionFixture extends Fixture {
  async launch(name, executable, env, argv = []) {
    if (name === 'kernel-A' || name === 'kernel-B') {
      const host = name.at(-1)
      const root = path.join(env.CHARIOX_HOME, 'state')
      await mkdir(root, {recursive: true, mode: 0o700})
      const cwd = await realpath(path.join(this.home, host, 'work'))
      await writeFile(path.join(cwd, 'binary-stream.py'), script, {mode: 0o600})
      // This is the ordinary learned-peak input to admission, not a gate override.
      // An impossible predicted peak makes the side-effect assertion red on the old kernel.
      const peaks = Object.fromEntries(['cancel', 'start', 'revoke', 'peer-start', 'peer-cancel', 'peer-revoke', 'peer-target-revoke', 'ui-start-standard', 'ui-cancel-standard', 'ui-start-retina', 'ui-cancel-retina', 'ui-owner-standard', 'ui-owner-retina', 'ui-place-standard', 'ui-place-retina', 'ui-stay-standard', 'ui-stay-retina'].map(name => [key(command(this.home, name, host), cwd), {memory: 2 ** 52, writes: 0}]))
      await writeFile(path.join(root, 'admission-peaks.json'), JSON.stringify(peaks), {mode: 0o600})
    }
    return super.launch(name, executable, env, argv)
  }
}
