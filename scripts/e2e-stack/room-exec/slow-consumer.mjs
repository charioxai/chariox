// MP-08 / MP-09 / MP-10 / MP-11: a real CLI pipeline stalled beyond the reader lease.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'
import { Fixture, until } from './fixture.mjs'

const args = process.argv.slice(2)
const value = flag => { const i = args.indexOf(flag); if (i < 0 || !args[i + 1]) throw new Error(`missing ${flag}`); return path.resolve(args[i + 1]) }
const fixture = new Fixture({ home: value('--home'), evidence: value('--evidence'), kernelBinary: value('--kernel-bin'), relayBinary: value('--relay-bin'), cliBinary: value('--cli-bin') })
const active = new Set()
const processes = []
let session
function signal(child) {
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1 || !active.has(child)) throw new Error('invalid owned pipeline PID')
  if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM')
}
function launch(executable, argv, env) {
  const child = spawn(executable, argv, { env, stdio: ['pipe', 'pipe', 'pipe'] })
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error('invalid owned pipeline PID')
  active.add(child)
  const row = { pid: child.pid, executable, code: null, signal: null }
  processes.push(row)
  child.stdin.on('error', () => undefined)
  child.done = new Promise((resolve, reject) => {
    child.once('error', reject)
    child.once('close', (code, signal) => { active.delete(child); Object.assign(row, { code, signal }); resolve({ code, signal }) })
  })
  child.done.catch(() => undefined)
  return child
}
const hash = bytes => createHash('sha256').update(bytes).digest('hex')
let deadline
try {
  await fixture.start()
  await fixture.grant()
  const work = path.join(fixture.home, 'B/work')
  session = (await fixture.B.send({ CreateSession: { workspace_id: work, worktree_id: work, alias: null, no_subscribe: true } })).SessionCreated.session.id
  // The downstream process reads one byte, then leaves a real OS pipe stalled
  // for longer than the production five-minute lease, before draining it.
  const consumer = launch('/usr/bin/python3', ['-c', `
import hashlib,json,sys,time
first=sys.stdin.buffer.read(1)
print('stalled',flush=True)
time.sleep(330)
data=first+sys.stdin.buffer.read()
print(json.dumps({'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()}),flush=True)
`], fixture.envs.A)
  let received = ''
  consumer.stdout.on('data', bytes => { received += bytes.toString() })
  consumer.stderr.on('data', () => undefined)
  const cli = launch(fixture.cliBinary, ['exec', '--socket', fixture.envs.A.CHARIOX_DAEMON_SOCKET,
    '--kernel', fixture.target, '--session', session, '--', '/usr/bin/python3', '-c',
    'import sys; sys.stderr.buffer.write(bytes(range(256))*4096); sys.stderr.buffer.flush(); sys.stdout.buffer.write(bytes(range(256))*32768); sys.stdout.buffer.flush(); sys.exit(23)'], fixture.envs.A)
  const errors = []
  cli.stderr.on('data', bytes => errors.push(bytes))
  cli.stdout.pipe(consumer.stdin)
  cli.stdin.end()
  deadline = setTimeout(() => { for (const child of active) signal(child) }, 420_000)
  await until(() => received.startsWith('stalled\n'), 'pipeline consumer stalled')
  console.log('consumer stalled for 330 seconds; production reader lease is 300 seconds')
  const started = Date.now()
  const [command, downstream] = await Promise.all([cli.done, consumer.done])
  const output = JSON.parse(received.trim().split('\n').at(-1))
  const stderr = Buffer.concat(errors)
  const pattern = Buffer.from(Array.from({ length: 256 }, (_, i) => i))
  const expected = count => hash(Buffer.concat(Array.from({ length: count }, () => pattern)))
  const result = { mp: ['MP-08', 'MP-09', 'MP-10', 'MP-11'], lease_seconds: 300, stall_seconds: 330,
    elapsed_ms: Date.now() - started, transport: 'native CLI -> origin kernel -> encrypted relay -> target kernel',
    command, downstream, stdout: output, stderr: { bytes: stderr.length, sha256: hash(stderr) },
    expected: { stdout_bytes: 8388608, stdout_sha256: expected(32768), stderr_bytes: 1048576, stderr_sha256: expected(4096), code: 23 } }
  await writeFile(path.join(fixture.evidence, 'results.json'), JSON.stringify(result, null, 2))
  assert.deepEqual(command, { code: 23, signal: null })
  assert.deepEqual(downstream, { code: 0, signal: null })
  assert.equal(output.bytes, result.expected.stdout_bytes)
  assert.equal(output.sha256, result.expected.stdout_sha256)
  assert.equal(stderr.length, result.expected.stderr_bytes)
  assert.equal(hash(stderr), result.expected.stderr_sha256)
  console.log('slow pipeline: PASS (all binary bytes and exit 23)')
} finally {
  clearTimeout(deadline)
  for (const child of active) signal(child)
  for (const child of [...active]) await child.done.catch(() => undefined)
  if (session) await fixture.B.send({ EndSession: { session_id: session } }).catch(() => undefined)
  await fixture.cleanup()
  await writeFile(path.join(fixture.evidence, 'pipeline-processes.json'), JSON.stringify(processes, null, 2))
}
