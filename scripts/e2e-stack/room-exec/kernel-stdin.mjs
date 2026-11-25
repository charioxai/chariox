// Run from the repository with CHARIOX_EXEC_{HOME,EVIDENCE,KERNEL,RELAY,CLI} set to absolute paths.
// Two disposable kernels and a scoped relay, each in its own systemd unit with an empty environment.
// Cells X1-X4 of the kernel exec stdin drill; every command goes through the native CLI.
import assert from 'node:assert/strict'
import { execFile, spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { openSync, closeSync } from 'node:fs'
import { readFile, writeFile } from 'node:fs/promises'
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

async function run(target, command, { input, seconds = 20, flags = [] } = {}) {
  const fd = input === undefined ? 'ignore' : openSync(input, 'r')
  const child = spawn(fixture.cliBinary, ['exec', '--socket', fixture.envs.A.CHARIOX_DAEMON_SOCKET,
    '--kernel', target, '--timeout', String(seconds), ...flags, '--', ...command],
  { env: fixture.envs.A, stdio: [fd, 'pipe', 'pipe'] })
  const stdout = [], stderr = []
  child.stdout.on('data', chunk => stdout.push(chunk))
  child.stderr.on('data', chunk => stderr.push(chunk))
  let timer
  try {
    const code = await new Promise((resolve, reject) => {
      timer = setTimeout(() => child.kill('SIGKILL'), 90_000)
      child.once('error', reject)
      child.once('close', resolve)
    })
    return { code, stdout: Buffer.concat(stdout).toString(), stderr: Buffer.concat(stderr).toString() }
  } finally {
    clearTimeout(timer)
    if (typeof fd === 'number') closeSync(fd)
  }
}

async function cell(id, exercise) {
  try { results.push({ id, status: 'PASS', ...await exercise() }) }
  catch (error) { results.push({ id, status: 'FAIL', message: String(error.message).slice(0, 600) }) }
  await writeFile(path.join(fixture.evidence, 'results.json'), JSON.stringify(results, null, 2) + '\n')
  console.log(id, results.at(-1).status, results.at(-1).message ?? '')
}

// A script of exactly `size` bytes: padding comments, then the work, ending with `exit <code>`.
function script(size, counter, code) {
  const tail = `echo "ran $(( 6 * 7 ))"; echo x >> '${counter}'; exit ${code}\n`
  const line = '# padding line that keeps the script far above the 4096 byte argument limit ......\n'
  let text = ''
  while (text.length + line.length + tail.length <= size) text += line
  text += '#'.repeat(size - text.length - tail.length - 1) + '\n' + tail
  assert.equal(text.length, size)
  return text
}

try {
  await fixture.start()
  await fixture.grant()
  await exec(fixture.cliBinary, ['exec', '--help'], { env: fixture.envs.A }).catch(() => undefined)
  for (const [label, target, root] of [['local', fixture.ids.A, 'A'], ['relay', fixture.target, 'B']]) {
    const counter = path.join(fixture.home, root, 'work', `counter-${label}`)
    const file = path.join(fixture.home, `script-${label}.sh`)
    const text = script(64 * 1024, counter, 7)
    await writeFile(file, text)
    await cell(`X${label === 'local' ? 1 : 4}-${label}`, async () => {
      const result = await run(target, ['bash', '-s'], { input: file })
      assert.equal(result.code, 7, JSON.stringify(result))
      assert.match(result.stdout, /^ran 42\n$/, JSON.stringify(result))
      assert.equal((await readFile(counter, 'utf8')), 'x\n', 'the script ran exactly once')
      return { script_bytes: text.length, sha256: createHash('sha256').update(text).digest('hex'), ...result }
    })
    await cell(`X2-file-${label}`, async () => {
      const remote = path.join(fixture.home, root, 'work', `uploaded-${label}.sh`)
      const put = await exec(fixture.cliBinary, ['files', 'put', '--socket', fixture.envs.A.CHARIOX_DAEMON_SOCKET, '--kernel', target, file, remote], { env: fixture.envs.A })
      const result = await run(target, ['bash', '-s'], { flags: ['--stdin-file', remote] })
      assert.equal(result.code, 7, JSON.stringify(result))
      assert.match(result.stdout, /^ran 42\n$/, JSON.stringify(result))
      assert.equal((await readFile(counter, 'utf8')), 'x\nx\n', 'the uploaded script ran once more')
      return { upload: JSON.parse(put.stdout), ...result }
    })
    await cell(`X3-${label}`, async () => {
      const long = 'a'.repeat(5000)
      const argument = await run(target, ['/bin/echo', long])
      assert.notEqual(argument.code, 0)
      const refusal = argument.stderr
      assert.match(refusal, /4096/, refusal)
      assert.match(refusal, /stdin/, refusal)
      assert.match(refusal, /stdin file|--stdin-file|files put/, refusal)
      const big = path.join(fixture.home, `big-${label}.sh`)
      await writeFile(big, 'x'.repeat(64 * 1024 + 1))
      const oversize = await run(target, ['bash', '-s'], { input: big })
      assert.notEqual(oversize.code, 0)
      assert.match(oversize.stderr, /64 KiB/, oversize.stderr)
      assert.match(oversize.stderr, /--stdin-file/, oversize.stderr)
      return { argument_refusal: refusal.slice(0, 400), stdin_refusal: oversize.stderr.slice(0, 400) }
    })
  }
  if (results.some(row => row.status !== 'PASS')) process.exitCode = 1
} finally {
  await fixture.cleanup()
}
