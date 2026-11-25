// Run from the repository with CHARIOX_EXEC_{HOME,EVIDENCE,KERNEL,RELAY,CLI} set to absolute paths.
// A command on a kernel must not wait for a stdin that stays open and idle (an agent's tool shell, cron),
// and stdin that is given (a file, a pipe with data, --stdin, --stdin-file) must still arrive.
import assert from 'node:assert/strict'
import { execFile, spawn } from 'node:child_process'
import { openSync, closeSync } from 'node:fs'
import { writeFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'
import { SystemdFixture } from './systemd-fixture.mjs'

const exec = promisify(execFile)
const envPath = name => path.resolve(process.env[`CHARIOX_EXEC_${name}`])
const fixture = new SystemdFixture({
  home: envPath('HOME'), evidence: envPath('EVIDENCE'), kernelBinary: envPath('KERNEL'),
  relayBinary: envPath('RELAY'), cliBinary: envPath('CLI'),
})
const results = []
const NORMAL_MS = 4000

function finish(child, hold) {
  const stdout = [], stderr = []
  child.stdout.on('data', chunk => stdout.push(chunk))
  child.stderr.on('data', chunk => stderr.push(chunk))
  const started = performance.now()
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { child.kill('SIGKILL'); reject(new Error('the CLI did not finish within 40 s')) }, 40_000)
    child.once('error', reject)
    child.once('close', code => {
      clearTimeout(timer)
      hold?.destroy()
      resolve({ code, stdout: Buffer.concat(stdout).toString(), stderr: Buffer.concat(stderr).toString(), elapsed_ms: Math.round(performance.now() - started) })
    })
  })
}

const cliArgs = (flags, command, seconds = 8) => ['exec', '--socket', fixture.envs.A.CHARIOX_DAEMON_SOCKET, '--kernel', fixture.ids.A, '--timeout', String(seconds), ...flags, '--', ...command]

// stdin: 'idle-pipe' (an open pipe nobody writes to), a file path (`< file`), or 'ignore'.
function run(stdin, flags, command) {
  if (stdin === 'idle-pipe') {
    const child = spawn(fixture.cliBinary, cliArgs(flags, command), { env: fixture.envs.A, stdio: ['pipe', 'pipe', 'pipe'] })
    return finish(child, child.stdin)
  }
  const fd = stdin === 'ignore' ? 'ignore' : openSync(stdin, 'r')
  const child = spawn(fixture.cliBinary, cliArgs(flags, command), { env: fixture.envs.A, stdio: [fd, 'pipe', 'pipe'] })
  return finish(child).finally(() => { if (typeof fd === 'number') closeSync(fd) })
}

// A shell line: the CLI's stdin comes from the pipeline or redirection in `line` (the CLI is $CLI).
function shell(line, flags, command) {
  const child = spawn('bash', ['-c', line, 'bash', ...cliArgs(flags, command)], { env: { ...fixture.envs.A, CLI: fixture.cliBinary }, stdio: ['ignore', 'pipe', 'pipe'] })
  return finish(child)
}

async function cell(id, exercise) {
  try { results.push({ id, status: 'PASS', ...await exercise() }) }
  catch (error) { results.push({ id, status: 'FAIL', message: String(error.message).slice(0, 600) }) }
  await writeFile(path.join(fixture.evidence, 'results.json'), JSON.stringify(results, null, 2) + '\n')
  console.log(id, results.at(-1).status, results.at(-1).message ?? '')
}

const quickAndRight = (result, expected) => {
  assert.equal(result.code, 0, JSON.stringify(result))
  assert.match(result.stdout, expected, JSON.stringify(result))
  assert.equal(result.stderr, '', `nothing may be printed on stderr: ${JSON.stringify(result)}`)
  assert.ok(result.elapsed_ms < NORMAL_MS, `took ${result.elapsed_ms} ms: ${JSON.stringify(result)}`)
}

try {
  await fixture.start()
  await fixture.grant()
  await exec(fixture.cliBinary, ['exec', '--help'], { env: fixture.envs.A }).catch(() => undefined)
  const hostname = ['bash', '-c', 'cat /etc/hostname']
  await cell('X1-claude-code-style-open-idle-stdin', async () => {
    const result = await run('idle-pipe', [], hostname)
    quickAndRight(result, /\S/)
    return result
  })
  await cell('X2-codex-style-pipe-held-open-by-parent', async () => {
    // The producer end stays open (a parent shell holding it); the CLI is not told it ended. The idle writer
    // keeps no output pipe of the drill open, so the CLI's own finish time is what is measured.
    const result = await shell('"$CLI" "$@" < <(exec sleep 25 2>/dev/null)', [], hostname)
    quickAndRight(result, /\S/)
    return result
  })
  await cell('X3-given-input-still-arrives', async () => {
    const piped = await shell('echo from-a-pipe | "$CLI" "$@"', [], ['cat'])
    quickAndRight(piped, /^from-a-pipe\n$/)
    const file = path.join(fixture.home, 'redirect.txt')
    await writeFile(file, 'from-a-file\n')
    const redirected = await run(file, [], ['cat'])
    quickAndRight(redirected, /^from-a-file\n$/)
    const slow = await shell('(sleep 1; echo slow-one) | "$CLI" "$@"', ['--stdin'], ['cat'])
    assert.equal(slow.code, 0, JSON.stringify(slow))
    assert.match(slow.stdout, /^slow-one\n$/, JSON.stringify(slow))
    assert.equal(slow.stderr, '', JSON.stringify(slow))
    return { piped, redirected, slow_with_stdin_flag: slow }
  })
  await cell('X4-stdin-file-unchanged', async () => {
    const local = path.join(fixture.home, 'input.txt')
    await writeFile(local, 'from-the-kernel-file\n')
    const remote = path.join(fixture.home, 'A/work/input.txt')
    await exec(fixture.cliBinary, ['files', 'put', '--socket', fixture.envs.A.CHARIOX_DAEMON_SOCKET, '--kernel', fixture.ids.A, local, remote], { env: fixture.envs.A })
    const result = await run('idle-pipe', ['--stdin-file', remote], ['cat'])
    quickAndRight(result, /^from-the-kernel-file\n$/)
    return result
  })
  await cell('X5-terminal-behaves-as-today', async () => {
    // `script` gives the CLI a pseudo-terminal as stdin: it is not read, the command runs at once.
    const line = `script -qec "$CLI exec --socket '${fixture.envs.A.CHARIOX_DAEMON_SOCKET}' --kernel '${fixture.ids.A}' --timeout 8 -- bash -c 'echo tty-ok'" /dev/null`
    const result = await finish(spawn('bash', ['-c', line], { env: { ...fixture.envs.A, CLI: fixture.cliBinary }, stdio: ['ignore', 'pipe', 'pipe'] }))
    assert.match(result.stdout, /tty-ok/, JSON.stringify(result))
    assert.ok(result.elapsed_ms < NORMAL_MS, JSON.stringify(result))
    return result
  })
  await cell('X6-late-input-is-reported-not-lost-silently', async () => {
    // A producer slower than the probe: the command runs without its input and one stderr line says so.
    // (`cat` would wait on the terminal for input, as it always has without stdin; this command ends by itself.)
    const result = await shell('(sleep 1; echo late) | "$CLI" "$@"', [], ['bash', '-c', 'sleep 3; echo ran'])
    assert.equal(result.code, 0, JSON.stringify(result))
    assert.equal(result.stdout, 'ran\n', JSON.stringify(result))
    assert.equal(result.stderr, 'stdin data arrived after the command started and was not sent; pass --stdin to wait for it\n', JSON.stringify(result))
    return result
  })
  await cell('X7-unreadable-stdin-is-an-error-not-a-crash', async () => {
    // A directory redirected into stdin cannot be read: one JSON error, a non-zero exit, no stack trace.
    const result = await shell('"$CLI" "$@" < /var/tmp', [], ['true'])
    assert.notEqual(result.code, 0, JSON.stringify(result))
    assert.equal(JSON.parse(result.stderr).error.code, 'exec_stdin_unreadable', JSON.stringify(result))
    assert.doesNotMatch(result.stderr, /\n\s+at |Unhandled|node:internal/, JSON.stringify(result))
    assert.ok(result.elapsed_ms < NORMAL_MS, `took ${result.elapsed_ms} ms: ${JSON.stringify(result)}`)
    return result
  })
  if (results.some(row => row.status !== 'PASS')) process.exitCode = 1
} finally {
  await fixture.cleanup()
}
