// Run from the repository with CHARIOX_EXEC_{HOME,EVIDENCE,KERNEL,RELAY,CLI} set to absolute paths.
// Two disposable kernels and a scoped relay, each in its own systemd unit with an empty environment.
// Cells C1-C5 of the "chariox exec survives a short relay drop" drill (queue row 424): the relay is stopped by its
// exact unit and started again while a command runs on kernel B and the native CLI on kernel A waits for it.
// CHARIOX_EXEC_TICKS sets the length of the C1 command in 10 s steps (60 = the 10-minute cell).
import assert from 'node:assert/strict'
import { execFile, spawn } from 'node:child_process'
import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { promisify } from 'node:util'
import { SystemdFixture } from './systemd-fixture.mjs'
import { until } from './fixture.mjs'

const exec = promisify(execFile)
const envPath = name => path.resolve(process.env[`CHARIOX_EXEC_${name}`])
const ticks = Number(process.env.CHARIOX_EXEC_TICKS ?? 60)
// CHARIOX_EXEC_ONLY=C1 runs that cell alone (the 10-minute run is longer than the units' 900 s with the others after it).
const only = process.env.CHARIOX_EXEC_ONLY ?? ''

const fixture = new SystemdFixture({
  home: envPath('HOME'), evidence: envPath('EVIDENCE'), kernelBinary: envPath('KERNEL'),
  relayBinary: envPath('RELAY'), cliBinary: envPath('CLI'),
})
const results = []
const sock = () => fixture.envs.A.CHARIOX_DAEMON_SOCKET

function native(args) {
  const split = args.includes('--') ? args.indexOf('--') : args.length
  const child = spawn(fixture.cliBinary, [...args.slice(0, split), '--socket', sock(), ...args.slice(split)], { env: fixture.envs.A, stdio: ['ignore', 'pipe', 'pipe'] })
  const stdout = [], stderr = []
  child.stdout.on('data', chunk => stdout.push(chunk))
  child.stderr.on('data', chunk => stderr.push(chunk))
  child.finished = new Promise((resolve, reject) => {
    const timer = setTimeout(() => child.kill('SIGKILL'), 1_500_000)
    child.once('error', reject)
    child.once('close', code => { clearTimeout(timer); resolve({ code, stdout: Buffer.concat(stdout).toString(), stderr: Buffer.concat(stderr).toString() }) })
  })
  return child
}
const run = async args => native(args).finished
const execOn = (seconds, script) => ['exec', '--kernel', fixture.target, '--timeout', String(seconds), '--', 'sh', '-c', script]

async function cell(id, exercise) {
  if (only && !id.startsWith(only)) return
  try { results.push({ id, status: 'PASS', ...await exercise() }) }
  catch (error) {
    results.push({ id, status: 'FAIL', message: String(error.message).slice(0, 900) })
    // A failed cell must not leave the relay down for the cells after it.
    if (!relayUnit) await restoreLink().catch(() => undefined)
  }
  await writeFile(path.join(fixture.evidence, 'results.json'), JSON.stringify(results, null, 2) + '\n')
  console.log(id, results.at(-1).status, results.at(-1).message ?? '')
}

// The relay link dropped and restored by the relay's own unit: the processes it owns are exactly the ones stopped.
let relayUnit
let relays = 0
async function dropLink() { await exec('systemctl', ['stop', relayUnit]); relayUnit = undefined }
async function restoreLink() {
  await fixture.launch(`relay-again-${++relays}`, fixture.relayBinary, {
    PATH: process.env.PATH, HOME: fixture.home, CHARIOX_HOME: path.join(fixture.home, 'relay'),
    CHARIOX_RELAY_HOST: '127.0.0.1', CHARIOX_RELAY_PORT: String(fixture.ports.relay),
    CHARIOX_RELAY_SCOPED_ISSUER: fixture.issuer, CHARIOX_RELAY_SCOPED_HMAC_SECRET: fixture.secret,
  })
  relayUnit = fixture.units.at(-1)
  for (const name of ['A', 'B']) await until(async () => { try { return (await fixture[name].send({ RelayStatus: null })).RelayStatus.status.connected } catch { return false } }, `kernel ${name} back on the relay`, 120_000)
}
// Terminals the target kernel still holds: an orphaned command shows here.
async function targetTerminals() {
  const listed = await run(['terminal', 'list', '--kernel', fixture.target])
  return { code: listed.code, text: listed.stdout, stderr: listed.stderr }
}
// The command has started on the target and the CLI has had time to read the open's answer: a drop while that answer
// is still in flight leaves the terminal id unknown, which is the open case (not retried), not what these cells exercise.
async function commandStarted(file) {
  await until(async () => (await lineCount(file)) === 1, 'command started on the target', 60_000)
  await delay(3_000)
}
const lineCount = async file => (await readFile(file, 'utf8').catch(() => '')).split('\n').filter(Boolean).length

try {
  await fixture.start()
  await fixture.grant()
  relayUnit = fixture.units[0]
  const work = path.join(fixture.home, 'B', 'work')

  await cell('C1-short-drop-long-exec', async () => {
    const started = path.join(work, 'c1-started')
    const body = `echo x >> '${started}'; i=1; while [ $i -le ${ticks} ]; do echo tick $i; i=$((i+1)); sleep 10; done; echo finished; exit 5`
    const child = native(execOn(ticks * 10 + 300, body))
    await commandStarted(started)
    await delay(30_000)
    const droppedAt = Date.now()
    await dropLink()
    await delay(30_000)
    await restoreLink()
    const outage = Math.round((Date.now() - droppedAt) / 1000)
    const result = await child.finished
    assert.equal(result.code, 5, JSON.stringify(result))
    const out = result.stdout.split('\n').filter(Boolean)
    assert.equal(out.at(-1), 'finished', 'the command ran to its end')
    assert.deepEqual(out.slice(0, ticks), Array.from({ length: ticks }, (_, i) => `tick ${i + 1}`), 'every tick, none lost or repeated')
    assert.equal(await lineCount(started), 1, 'the command ran exactly once')
    const after = await targetTerminals()
    assert.doesNotMatch(after.text, /c1-started/, 'nothing of it is left on the target')
    assert.match(result.stderr, /link to kernel .* lost, waiting for it; the command keeps running as terminal /, result.stderr)
    assert.equal((result.stderr.match(/waiting for it/g) ?? []).length, 1, 'one line, not one per try')
    return { outage_seconds: outage, exit: result.code, ticks, stderr: result.stderr.trim(), terminals_after: after.text.trim().slice(0, 400) }
  })

  await cell('C2-link-down-past-timeout', async () => {
    const started = path.join(work, 'c2-started')
    const child = native(execOn(20, `echo x >> '${started}'; sleep 60; echo late; exit 4`))
    await commandStarted(started)
    await dropLink()
    const result = await child.finished
    assert.notEqual(result.code, 0)
    const failure = JSON.parse(result.stderr.trim().split('\n').filter(line => line.startsWith('{')).at(-1)).error
    assert.equal(failure.code, 'exec_link_lost', result.stderr)
    assert.match(failure.message, /not stopped/)
    const terminal = /terminal (\S+)\./.exec(failure.message)?.[1] ?? failure.details?.terminal_id
    assert.ok(terminal, 'names the terminal')
    assert.doesNotMatch(result.stderr, /exec_terminal_left_open/)
    await restoreLink()
    // The command's fate is findable with the verbs the message names.
    const waited = await run(['terminal', 'wait', '--kernel', fixture.target, '--terminal', terminal, '--for', 'finished', '--timeout', '90'])
    assert.equal(waited.code, 0, JSON.stringify(waited))
    const read = await run(['terminal', 'read', '--kernel', fixture.target, '--terminal', terminal])
    assert.match(read.stdout, /late/, JSON.stringify(read))
    assert.equal(await lineCount(started), 1)
    return { failure: failure.message, terminal, wait: waited.stdout.trim().slice(0, 500), read: read.stdout.trim().slice(0, 300) }
  })

  await cell('C4-ctrl-c-during-drop', async () => {
    const started = path.join(work, 'c4-started')
    const child = native(execOn(120, `echo x >> '${started}'; sleep 100; echo never`))
    await commandStarted(started)
    await dropLink()
    await delay(8_000)
    child.kill('SIGINT')
    const result = await child.finished
    assert.equal(result.code, 130, JSON.stringify(result))
    await restoreLink()
    const named = /"terminal_id":"([^"]+)"/.exec(result.stderr)?.[1]
    // The close could not reach the kernel while the link was down: the terminal is named, and closing it by that name works.
    let closed = null
    if (named) closed = await run(['terminal', 'close', '--kernel', fixture.target, '--terminal', named])
    const after = await targetTerminals()
    assert.equal(after.text.includes('c4-started') || /sleep 100/.test(after.text), false, `no orphan left: ${after.text}`)
    return { exit: result.code, stderr: result.stderr.trim().slice(0, 500), named_terminal: named ?? null, close: closed?.stdout.trim().slice(0, 200) ?? null }
  })

  await cell('C5-finished-while-down', async () => {
    const started = path.join(work, 'c5-started')
    const child = native(execOn(120, `echo x >> '${started}'; sleep 8; echo fin; exit 3`))
    await commandStarted(started)
    await dropLink()
    await delay(20_000)
    await restoreLink()
    const result = await child.finished
    assert.equal(result.code, 3, JSON.stringify(result))
    assert.equal(result.stdout, 'fin\n')
    assert.equal(await lineCount(started), 1)
    return { exit: result.code, stdout: result.stdout, stderr: result.stderr.trim() }
  })

  if (results.some(row => row.status !== 'PASS')) process.exitCode = 1
} finally {
  await fixture.cleanup()
}
