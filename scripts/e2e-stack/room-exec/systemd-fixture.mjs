// Disposable drill kernels and relay, each in its own systemd unit with an empty environment.
import { execFile } from 'node:child_process'
import path from 'node:path'
import { promisify } from 'node:util'
import { Fixture } from './fixture.mjs'

const exec = promisify(execFile)

export class SystemdFixture extends Fixture {
  units = []

  async launch(name, executable, env, args = []) {
    const unit = `chariox-execstdin-${process.pid}-${name}.service`
    await exec('systemd-run', ['--quiet', '--unit', unit, '--property=Type=exec',
      '--property=RuntimeMaxSec=900', '--property=KillMode=control-group',
      `--property=StandardOutput=append:${path.join(this.evidence, name + '.log')}`,
      '--property=StandardError=inherit', '/usr/bin/env', '-i',
      ...Object.entries(env).map(([key, value]) => `${key}=${value}`), executable, ...args])
    this.units.push(unit)
  }

  async cleanup() {
    try {
      // Each transient unit owns exactly the process started for this drill.
      for (const unit of this.units.reverse()) await exec('systemctl', ['stop', unit]).catch(() => undefined)
    } finally {
      await super.cleanup().catch(() => undefined)
    }
  }
}
