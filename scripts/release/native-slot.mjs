import { chmodSync, closeSync, copyFileSync, openSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { run } from './process.mjs'

const source = process.env.CHARIOX_RELEASE_SOURCE, output = process.env.CHARIOX_RELEASE_OUTPUT
if (!source || !output || !process.env.CHARIOX_BUILD_SLOT || !process.env.CARGO_TARGET_DIR) throw Error('must run inside coordinator Cargo slot')
// Force the shared target to rebuild this checkout; touching does not change Git.
run('find', ['apps', 'packages', '-name', '*.rs', '-exec', 'touch', '{}', '+'], { cwd: source })
const log = join(output, 'cargo.json')
// JSON output and copying stay inside the slot, so another checkout cannot win.
const { spawnSync } = await import('node:child_process')
const descriptor = openSync(log, 'wx', 0o600)
let result
try {
  result = spawnSync('cargo', ['build', '--locked', '--release', '-p', 'chariox-kernel', '--bin', 'chariox-kernel',
    '-p', 'chariox-relay', '--bin', 'chariox-relay', '--message-format=json-render-diagnostics'],
  { cwd: source, stdio: ['ignore', descriptor, 'inherit'] })
} finally { closeSync(descriptor) }
if (result.error || result.status !== 0) throw Error('Cargo build failed')
const found = new Set()
for (const line of readFileSync(log, 'utf8').split('\n')) {
  if (!line) continue
  const item = JSON.parse(line)
  if (item.reason !== 'compiler-artifact' || !item.executable || !['chariox-kernel', 'chariox-relay'].includes(item.target.name)) continue
  if (!resolve(item.target.src_path).startsWith(`${source}/`)) throw Error('artifact belongs to another worktree')
  const destination = join(output, item.target.name)
  copyFileSync(item.executable, destination); chmodSync(destination, 0o755)
  found.add(item.target.name)
}
if (found.size !== 2) throw Error('missing exact-worktree native artifact')
