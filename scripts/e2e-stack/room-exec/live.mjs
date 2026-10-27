// MP-08 / MP-09 / MP-10 / MP-11: the native CLI through two real kernels and a scoped relay.
import assert from 'node:assert/strict'
import { spawn, execFile } from 'node:child_process'
import { once } from 'node:events'
import { createHash, randomBytes } from 'node:crypto'
import { createReadStream, createWriteStream } from 'node:fs'
import { mkdir, writeFile, readFile, rm } from 'node:fs/promises'
import { Writable } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import { promisify } from 'node:util'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { Fixture, until } from './fixture.mjs'

const args = process.argv.slice(2)
const value = flag => { const index = args.indexOf(flag); if (index < 0 || !args[index + 1]) throw new Error(`missing ${flag}`); return path.resolve(args[index + 1]) }
const fixture = new Fixture({ home: value('--home'), evidence: value('--evidence'), kernelBinary: value('--kernel-bin'), relayBinary: value('--relay-bin'), cliBinary: value('--cli-bin') })
const results = { mp: ['MP-08', 'MP-09', 'MP-10', 'MP-11'], transport: 'Unix grant -> origin kernel -> scoped encrypted relay -> target kernel', cells: [] }
const active = new Set()
const exec = promisify(execFile)
const streams = new Set()
const text = bytes => Buffer.concat(bytes).toString('utf8')

function signal(child, signal) {
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error('invalid owned CLI PID')
  if (child.exitCode === null && child.signalCode === null) child.kill(signal)
}

const startedProcesses = []
function track(child, kind) {
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error('invalid owned process PID')
  active.add(child)
  const row = { kind, pid: child.pid, code: null, signal: null }
  startedProcesses.push(row)
  child.done = new Promise((resolve, reject) => {
    child.once('error', reject)
    child.once('close', (code, signal) => {
      row.code = code; row.signal = signal
      active.delete(child); resolve({ code, signal })
    })
  })
  child.done.catch(() => undefined)
  const deadline = setTimeout(() => signal(child, 'SIGTERM'), 120_000)
  child.done.finally(() => clearTimeout(deadline)).catch(() => undefined)
  child.stdin?.on('error', () => undefined)
  return child
}

function cli(argv) {
  return track(spawn(fixture.cliBinary, argv, { env: fixture.envs.A, stdio: ['pipe', 'pipe', 'pipe'] }), 'cli')
}

function command(argv, extra = []) {
  return cli(['exec', '--socket', fixture.envs.A.CHARIOX_DAEMON_SOCKET,
    '--kernel', fixture.target, ...extra, '--', ...argv])
}

function capture(child) {
  const out = [], err = []
  let size = 0
  for (const [stream, target] of [[child.stdout, out], [child.stderr, err]]) stream.on('data', chunk => {
    size += chunk.length
    if (size > 1024 * 1024) { signal(child, 'SIGTERM'); return }
    target.push(chunk)
  })
  return child.done.then(status => ({ ...status, stdout: text(out), stderr: text(err) }))
}

async function run(argv, input = Buffer.alloc(0)) {
  const child = command(argv, ['--workspace', path.join(fixture.home, 'B/work')])
  const result = capture(child)
  child.stdin.end(input)
  return result
}

async function cell(id, test) {
  const started = Date.now()
  try {
    const detail = await test()
    results.cells.push({ id, status: 'PASS', elapsed_ms: Date.now() - started, ...detail })
    console.log(`${id}: PASS`)
  } catch (error) {
    results.cells.push({ id, status: 'FAIL', elapsed_ms: Date.now() - started, message: error.message })
    throw error
  } finally { await writeFile(path.join(fixture.evidence, 'results.json'), JSON.stringify(results, null, 2)) }
}

async function digest(file) {
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(file)) hash.update(chunk)
  return hash.digest('hex')
}

try {
  await fixture.start()
  const work = path.join(fixture.home, 'B/work')
  const session = (await fixture.B.send({ CreateSession: { workspace_id: work, worktree_id: work, alias: null, no_subscribe: true } })).SessionCreated.session.id
  await cell('g-no-terminal-authority', async () => {
    const terminal = cli(['terminal', 'open', '--socket', fixture.envs.A.CHARIOX_DAEMON_SOCKET, '--kernel', fixture.target, '--session', session])
    const terminalResult = capture(terminal); terminal.stdin.end()
    const exec = command(['/bin/cat'], ['--session', session])
    const execResult = capture(exec); exec.stdin.end()
    const [t, e] = await Promise.all([terminalResult, execResult])
    assert.ok(t.code !== 0 && e.code !== 0)
    const tc = JSON.parse(t.stderr.trim()).error.code, ec = JSON.parse(e.stderr.trim()).error.code
    assert.ok(tc.startsWith('kernel_access_') && ec.startsWith('kernel_access_'))
    return { terminal_code: tc, exec_code: ec, note: 'The product has whole-kernel grants; without a live grant neither terminal nor exec is admitted.' }
  })
  await fixture.B.send({ EndSession: { session_id: session } })
  await fixture.grant()
  console.log('disposable grant and gateway ready')

  const input = path.join(fixture.home, 'binary-input'), output = path.join(fixture.home, 'binary-output')
  await cell('a-200MiB-binary-roundtrip', async () => {
    const fd = createWriteStream(input, { mode: 0o600 })
    streams.add(fd)
    const bytes = 200 * 1024 * 1024
    const hash = createHash('sha256')
    for (let count = 0; count < bytes; count += 256 * 1024) {
      const chunk = randomBytes(256 * 1024); hash.update(chunk)
      if (!fd.write(chunk)) await once(fd, 'drain')
    }
    await new Promise((resolve, reject) => { fd.once('error', reject); fd.end(resolve) })
    streams.delete(fd)
    const expected = hash.digest('hex')
    const child = command(['/bin/cat'], ['--workspace', work])
    const errors = []; child.stderr.on('data', chunk => errors.push(chunk))
    await Promise.all([pipeline(createReadStream(input), child.stdin), pipeline(child.stdout, createWriteStream(output, { mode: 0o600 }))])
    const status = await child.done
    assert.equal(status.code, 0, text(errors))
    const actual = await digest(output)
    assert.equal(actual, expected)
    return { size_bytes: bytes, input_sha256: expected, output_sha256: actual, exit_code: status.code }
  })
  await rm(input); await rm(output)

  await cell('b-early-output', async () => {
    const started = Date.now()
    const child = command(['/bin/bash', '-c', 'printf "first\\n"; sleep 2; printf "last\\n"'], ['--workspace', work])
    let first, exited = false
    child.once('exit', () => { exited = true })
    child.stdout.once('data', () => { first = { at_ms: Date.now() - started, before_exit: !exited } })
    const result = capture(child); child.stdin.end()
    const ended = await result
    assert.equal(ended.code, 0, ended.stderr)
    assert.equal(ended.stdout, 'first\nlast\n')
    assert.ok(first?.before_exit)
    assert.ok(Date.now() - started - first.at_ms > 1000)
    return { first_output: first, total_ms: Date.now() - started }
  })

  await cell('c-interactive-exchange', async () => {
    const child = command(['/bin/bash', '-c', 'while IFS= read -r line; do printf "reply:%s\\n" "$line"; done'], ['--workspace', work])
    let received = ''; const errors = []
    child.stdout.on('data', chunk => { received += chunk.toString() })
    child.stderr.on('data', chunk => errors.push(chunk))
    const exchanges = []
    for (const line of ['one', 'two', 'three']) {
      const started = Date.now(); child.stdin.write(`${line}\n`)
      await until(() => received.includes(`reply:${line}\n`), `interactive ${line}`)
      assert.equal(child.exitCode, null)
      exchanges.push({ line, response_before_exit: true, elapsed_ms: Date.now() - started })
    }
    child.stdin.end()
    assert.equal((await child.done).code, 0, text(errors))
    return { exchanges }
  })

  await cell('d-stderr-exit-and-signal', async () => {
    const code = await run(['/bin/bash', '-c', 'printf out; printf err >&2; exit 23'])
    assert.equal(code.code, 23); assert.equal(code.stdout, 'out'); assert.equal(code.stderr, 'err')
    const signal = await run(['/bin/bash', '-c', 'kill -TERM $$'])
    assert.equal(signal.code, 143)
    return { exit_code: code.code, stdout: code.stdout, stderr: code.stderr, signal_exit_code: signal.code }
  })

  await cell('e-git-bundle-and-tar-streams', async () => {
    const repo = path.join(fixture.home, 'bundle-repo'), extracted = path.join(work, 'unpacked')
    await mkdir(repo); await mkdir(extracted)
    await exec('git', ['init', '-q', repo])
    await writeFile(path.join(repo, 'readme'), 'binary exec git bundle\n')
    await exec('git', ['-C', repo, 'add', 'readme'])
    await exec('git', ['-C', repo, '-c', 'user.name=drill', '-c', 'user.email=drill@example.invalid', 'commit', '-qm', 'initial'])
    const base = (await exec('git', ['-C', repo, 'rev-parse', 'HEAD'])).stdout.trim()
    await writeFile(path.join(repo, 'readme'), 'updated through the live stream\n')
    await exec('git', ['-C', repo, '-c', 'user.name=drill', '-c', 'user.email=drill@example.invalid', 'commit', '-qam', 'update'])
    const bundle = path.join(fixture.home, 'change.bundle'), remoteBundle = path.join(work, 'original.bundle')
    const child = command(['/bin/bash', '-c', 'set -o pipefail; git -C "$1" bundle create - "$2"..HEAD | tee "$3"', 'bundle', repo, base, remoteBundle], ['--workspace', work])
    const errors = []; child.stderr.on('data', chunk => errors.push(chunk)); child.stdin.end()
    await pipeline(child.stdout, createWriteStream(bundle))
    assert.equal((await child.done).code, 0, text(errors))
    await exec('git', ['-C', repo, 'bundle', 'verify', bundle])
    const sha = await digest(bundle)
    const remote = await run(['sha256sum', remoteBundle])
    assert.equal(remote.code, 0); assert.equal(remote.stdout.split(' ')[0], sha)
    const tar = track(spawn('tar', ['-cf', '-', '-C', fixture.home, 'change.bundle'], { stdio: ['ignore', 'pipe', 'pipe'] }), 'tar')
    tar.stderr.resume()
    const extract = command(['tar', '-xf', '-', '-C', extracted], ['--workspace', work])
    const done = capture(extract)
    await pipeline(tar.stdout, extract.stdin)
    assert.equal((await tar.done).code, 0); assert.equal((await done).code, 0)
    const checked = await run(['sha256sum', path.join(extracted, 'change.bundle')])
    assert.equal(checked.code, 0); assert.equal(checked.stdout.split(' ')[0], sha)
    return { base, bundle_sha256: sha, remote_sha256: sha, extracted_sha256: sha }
  })

  await cell('f-cancel-midstream', async () => {
    const child = command(['/bin/bash', '-c', 'exec yes'], ['--workspace', work])
    let bytes = 0
    child.stdout.on('data', chunk => { bytes += chunk.length })
    const errors = []; child.stderr.on('data', chunk => errors.push(chunk)); child.stdin.end()
    await until(() => bytes > 1024 * 1024, 'live output before cancellation')
    const started = Date.now(); signal(child, 'SIGINT')
    const result = await child.done
    assert.equal(result.code, 130, text(errors))
    return { bytes_before_cancel: bytes, settled_ms: Date.now() - started, exit_code: result.code }
  })

  await cell('h-slow-reader-bounded-duplex', async () => {
    const child = command(['/bin/cat'], ['--workspace', work])
    const kernel = fixture.children.find(row => row.name === 'kernel-B').child.pid
    const rss = async () => Number((await readFile(`/proc/${kernel}/status`, 'utf8')).match(/^VmRSS:\s+(\d+)/m)[1]) * 1024
    const samples = [await rss()]
    let bytes = 0
    child.stdout.pause()
    child.stderr.resume()
    const sending = pipeline((async function* () { for (let count = 0; count < 256; count++) yield Buffer.alloc(256 * 1024) })(), child.stdin)
    sending.catch(() => undefined)
    for (let count = 0; count < 12; count++) { await delay(250); samples.push(await rss()) }
    assert.equal(child.exitCode, null, 'slow reader must backpressure the running command')
    await pipeline(child.stdout, new Writable({ write(chunk, _encoding, done) { bytes += chunk.length; done() } }))
    await sending
    assert.equal((await child.done).code, 0)
    assert.equal(bytes, 64 * 1024 * 1024)
    const growth = Math.max(...samples) - samples[0]
    assert.ok(growth < 32 * 1024 * 1024, `slow-reader RSS grew by ${growth} bytes`)
    return { bytes, paused_ms: 3000, kernel_rss_samples: samples, growth_bytes: growth }
  })
  results.status = 'PASS'
} finally {
  for (const stream of streams) stream.destroy()
  for (const child of active) { signal(child, 'SIGTERM'); child.stdin?.destroy(); child.stdout?.destroy(); child.stderr?.destroy() }
  await Promise.allSettled([...active].map(child => child.done))
  await fixture.cleanup()
  results.processes = startedProcesses
  await writeFile(path.join(fixture.evidence, 'results.json'), JSON.stringify(results, null, 2))
}
