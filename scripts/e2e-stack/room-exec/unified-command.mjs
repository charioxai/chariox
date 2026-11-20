import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { once } from 'node:events'
import { access, writeFile } from 'node:fs/promises'
import { userInfo } from 'node:os'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { Fixture } from './fixture.mjs'

const flags = new Map()
for (let index = 2; index < process.argv.length; index += 2) {
  flags.set(process.argv[index], process.argv[index + 1])
}
function file(flag) {
  assert.ok(flags.get(flag), `missing ${flag}`)
  return path.resolve(flags.get(flag))
}
const fixture = new Fixture({
  home: file('--home'), evidence: file('--evidence'), kernelBinary: file('--kernel-bin'),
  relayBinary: file('--relay-bin'), cliBinary: file('--cli-bin'), systemdScopes: true,
})
const results = []
const remote = () => ['--kernel', fixture.target]
const localWork = path.join(fixture.home, 'A/work')
const remoteWork = path.join(fixture.home, 'B/work')
const noInput = Symbol('leave the terminal open')
const noDirectNetwork = ['--quiet', '--scope', '--property', 'IPAddressDeny=any', '--']
const fixtureBytes = Buffer.alloc(320 * 1024)
for (let i = 0; i < fixtureBytes.length; i++) fixtureBytes[i] = i % 256

const stdout = Buffer.from([0, 13, 10, 255])
const stderr = Buffer.from([0, 13, 10, 254])
const byteCommand = ['/bin/sh', '-c', "printf '\\000\\r\\n\\377'; printf '\\000\\r\\n\\376' >&2; exit 7"]
const echoCommand = ['/bin/sh', '-c', 'printf ready >&2; exec cat']
const quote = word => "'" + word.replaceAll("'", "'\\''") + "'"

async function cli(target, argv, { input = Buffer.alloc(0), seconds = 8 } = {}) {
  const args = ['exec', '--socket', fixture.envs.A.CHARIOX_DAEMON_SOCKET, '--timeout', String(seconds), ...target, '--', ...argv]
  const tty = input === noInput
  let program = fixture.cliBinary
  let argvWithOptions = args
  if (tty) {
    program = '/usr/bin/script'
    argvWithOptions = ['-qefc', [fixture.cliBinary, ...args].map(quote).join(' '), '/dev/null']
  }
  if (fixture.target && target.includes(fixture.target)) {
    argvWithOptions = [...noDirectNetwork, program, ...argvWithOptions]
    program = '/usr/bin/systemd-run'
  }
  const child = spawn(program, argvWithOptions, { env: fixture.envs.A, stdio: ['pipe', 'pipe', 'pipe'] })
  const out = []
  const err = []
  child.stdout.on('data', data => out.push(data))
  child.stderr.on('data', data => err.push(data))
  child.stdin.on('error', error => {
    if (error.code !== 'EPIPE') child.emit('error', error)
  })
  if (!tty) child.stdin.end(input)
  const started = performance.now()
  const bound = setTimeout(() => child.kill('SIGKILL'), (seconds + 15) * 1000)
  try {
    const [code, signal] = await once(child, 'close')
    assert.equal(signal, null, 'CLI exceeded its time bound')
    return { code, stdout: Buffer.concat(out), stderr: Buffer.concat(err), elapsed: performance.now() - started }
  } finally {
    clearTimeout(bound)
    child.stdin.destroy()
  }
}

async function cell(id, run) {
  const started = performance.now()
  try {
    await run()
    results.push({ id, status: 'PASS', elapsed_ms: Math.round(performance.now() - started) })
    console.log(`${id} PASS`)
  } catch (error) {
    results.push({ id, status: 'FAIL', reason: String(error).slice(0, 500) })
    console.error(`${id} FAIL ${String(error).slice(0, 500)}`)
  }
}

async function binary(target) {
  const reply = await cli(target, byteCommand)
  assert.equal(reply.code, 7, reply.stderr.toString())
  assert.ok(reply.stdout.equals(stdout), `stdout bytes ${reply.stdout.toString('hex')}`)
  assert.ok(reply.stderr.equals(stderr), `stderr bytes ${reply.stderr.toString('hex')}`)
}

async function duplex(target) {
  const reply = await cli(target, echoCommand, { input: fixtureBytes })
  assert.equal(reply.code, 0, reply.stderr.toString())
  assert.equal(reply.stderr.toString(), 'ready')
  assert.ok(reply.stdout.equals(fixtureBytes), `stdout length ${reply.stdout.length}`)
}

async function deadline(target, sentinel) {
  const reply = await cli(target, ['/bin/sh', '-c', 'printf began; sleep 3; touch "$1"', 'drill', sentinel], { seconds: 1 })
  assert.notEqual(reply.code, 0)
  assert.ok(reply.stdout.equals(Buffer.from('began')))
  assert.match(reply.stderr.toString(), /didn.t finish.*1 s/i)
  assert.match(reply.stderr.toString(), /stopped.*(?:exit|signal)/i)
  assert.ok(reply.elapsed < 2800, `deadline returned after ${reply.elapsed} ms`)
  await delay(2200)
  await assert.rejects(access(sentinel), { code: 'ENOENT' })
}

async function counts(client) {
  return {
    projects: (await client.send({ ListProjects: {} })).ProjectsListed.projects.length,
    rooms: (await client.send({ ListSessions: {} })).SessionsListed.sessions.length,
  }
}

try {
  await fixture.start()

  await cell('U6-ungranted', async () => {
    const marker = path.join(localWork, 'run-without-access')
    const reply = await cli([], ['/bin/touch', marker])
    assert.equal(reply.stdout.length, 0)
    assert.notEqual(reply.code, 0)
    assert.match(reply.stderr.toString(), /kernel_access_not_covered|no.+grant/i)
    await assert.rejects(access(marker), { code: 'ENOENT' })
  })
  await fixture.grant()

  await cell('U5', async () => {
    assert.deepEqual(await counts(fixture.B), { projects: 0, rooms: 0 })
    const defaultDir = await cli(remote(), ['/bin/pwd'])
    const explicitDir = await cli([...remote(), '--workspace', remoteWork], ['/bin/pwd'])
    assert.equal(defaultDir.code, 0, defaultDir.stderr.toString())
    assert.equal(defaultDir.stdout.toString(), userInfo().homedir + '\n')
    assert.equal(explicitDir.code, 0, explicitDir.stderr.toString())
    assert.equal(explicitDir.stdout.toString(), remoteWork + '\n')
    assert.deepEqual(await counts(fixture.B), { projects: 0, rooms: 0 })
  })

  await cell('U7', async () => {
    // The CLI can use its Unix socket; the selected kernel's TCP listener is unreachable from its scope.
    const probe = "const s = require('node:net').connect(Number(process.argv[1]), '127.0.0.1'); s.once('connect', () => process.exit(1)); s.once('error', () => process.exit(0)); s.setTimeout(1000, () => process.exit(0))"
    execFileSync('/usr/bin/systemd-run', [...noDirectNetwork, process.execPath, '-e', probe, String(fixture.ports.B)], {
      env: fixture.envs.A, timeout: 5000,
    })
    const selected = [...remote(), '--workspace', remoteWork]
    await binary(selected)
    await duplex(selected)
    await deadline(selected, path.join(remoteWork, 'after-deadline'))
    const where = await cli(selected, ['/usr/bin/printenv', 'CHARIOX_DAEMON_ALIAS'])
    assert.equal(where.stdout.toString(), fixture.envs.B.CHARIOX_DAEMON_ALIAS + '\n')
  })

  const { session, agent } = (await fixture.A.send({
    CreateSession: { workspace_id: localWork, worktree_id: localWork, no_subscribe: true },
  })).SessionCreated
  const room = ['--session', session.id]
  const targets = [[], room, [...room, '--host-agent', agent.id]]
  await cell('U1', async () => { for (const target of targets) await binary(target) })
  await cell('U2', async () => { for (const target of targets) await duplex(target) })
  await cell('U3', async () => {
    for (const target of targets) {
      const reply = await cli(target, ['/bin/sh', '-c', 'cat; printf eof'], { input: noInput, seconds: 2 })
      assert.equal(reply.code, 0, reply.stderr.toString())
      assert.ok(reply.stdout.includes('eof'))
      assert.ok(reply.elapsed < 1500, `EOF took ${reply.elapsed} ms`)
    }
  })
  await cell('U4', async () => {
    for (const [index, target] of targets.entries()) {
      await deadline(target, path.join(localWork, `after-deadline-${index}`))
    }
  })
  await cell('U6-granted', async () => {
    await binary(targets[2])
    const marker = path.join(localWork, 'run-in-wrong-slice')
    const refused = await cli([...room, '--host-agent', 'not-in-this-room'], ['/bin/touch', marker])
    assert.notEqual(refused.code, 0)
    assert.match(refused.stderr.toString(), /no agent.*not-in-this-room/i)
    await assert.rejects(access(marker), { code: 'ENOENT' })
  })
} finally {
  await fixture.cleanup()
  await writeFile(path.join(fixture.evidence, 'unified-command-results.json'), JSON.stringify(results, null, 2))
}
if (results.some(result => result.status !== 'PASS')) process.exitCode = 1
