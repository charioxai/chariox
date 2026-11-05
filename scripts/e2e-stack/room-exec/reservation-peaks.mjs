// MP-08 / MP-09 / MP-10 / MP-11: native binary admission and learned peaks.
import assert from 'node:assert/strict'
import {spawn} from 'node:child_process'
import {createHash} from 'node:crypto'
import {mkdir, readFile, readdir, rmdir, writeFile} from 'node:fs/promises'
import path from 'node:path'
import {LocalIpcClient} from '../../../packages/kernel-client/dist/ipc.js'
import {Fixture, until} from './fixture.mjs'

const args = process.argv.slice(2)
const value = flag => {const at = args.indexOf(flag); assert.ok(at >= 0 && args[at + 1], `missing ${flag}`); return path.resolve(args[at + 1])}
const home = value('--home'), evidence = value('--evidence'), mib = 1048576
const cgroup = `/sys/fs/cgroup/chariox-p1runon-${process.pid}`
const mode = args.includes('--only-peaks') ? 'peaks' : args.includes('--only-reservations') ? 'reservations' : 'both'
const key = (argv, cwd) => {
  const hash = createHash('sha256')
  for (const argument of argv) {hash.update(argument.replace(/[a-fA-F0-9]+/g, run => run.length >= 6 || /^\d+$/.test(run) ? '#' : run)); hash.update(Buffer.from([0]))}
  hash.update(cwd); return hash.digest('hex').slice(0, 32)
}
const program = 'import sys; n=int(sys.stdin.buffer.readline()); b=bytearray(n*1048576); print("ready",flush=True); sys.stdin.buffer.readline()'
const command = name => ['/usr/bin/python3', '-c', program, name]
class AdmissionFixture extends Fixture {
  async launch(name, binary, env, argv = []) {
    if (name === 'kernel-A') {
      const state = path.join(env.CHARIOX_HOME, 'state'), cwd = path.join(home, 'A/work')
      await mkdir(state, {recursive: true})
      await writeFile(path.join(state, 'admission-peaks.json'), JSON.stringify(Object.fromEntries([
        ['reserved-work', 2048 * mib], ['pressure-work', 4608 * mib],
      ].map(([name, memory]) => [key(command(name), cwd), {memory, writes: 0}]))))
    }
    if (name.startsWith('kernel-')) {
      const group = path.join(cgroup, name)
      await mkdir(group)
      return super.launch(name, '/bin/bash', env, ['-c', 'echo "$$" > "$1"; shift; exec "$@"', 'binary-admission', path.join(group, 'cgroup.procs'), binary, ...argv])
    }
    return super.launch(name, binary, env, argv)
  }
}
const fixture = new AdmissionFixture({home, evidence, kernelBinary: value('--kernel-bin'), relayBinary: value('--relay-bin'), cliBinary: value('--cli-bin')})
const active = new Set(), cells = []
const result = {mp: ['MP-08', 'MP-09', 'MP-10', 'MP-11'], status: 'FAIL', scope: 'native CLI, real kernels and local scoped relay in a 6 GiB cgroup; no hosted or managed-placement acceptance', cells}
let session, ownsCgroup = false
function signal(child, name) {
  assert.ok(Number.isSafeInteger(child.pid) && child.pid > 1, 'invalid owned PID')
  if (child.exitCode === null && child.signalCode === null) child.kill(name)
}
function run(name, memory) {
  const child = spawn(fixture.cliBinary, ['exec', '--socket', fixture.envs.A.CHARIOX_DAEMON_SOCKET, '--session', session, '--', ...command(name)], {env: fixture.envs.A, stdio: ['pipe', 'pipe', 'pipe']})
  assert.ok(Number.isSafeInteger(child.pid) && child.pid > 1, 'invalid native PID')
  active.add(child)
  child.output = ''; child.errors = ''
  child.stdout.on('data', bytes => {child.output += bytes})
  child.stderr.on('data', bytes => {child.errors += bytes})
  child.stdin.on('error', () => {})
  child.done = new Promise((resolve, reject) => {child.once('error', reject); child.once('close', (code, signal) => {active.delete(child); resolve({code, signal})})})
  child.done.catch(() => {})
  child.stdin.write(`${memory}\n`)
  return child
}
async function ready(child) {await until(() => child.output === 'ready\n', 'native binary ready', 15_000)}
async function finish(child) {child.stdin.end('done\n'); assert.equal((await child.done).code, 0, child.errors)}
async function listed() {return (await fixture.A.send({RoomTerminal: {session_id: session, operation: {operation: 'exec_list'}}})).RoomTerminal.result.execs}
async function cell(id, work) {
  try {cells.push({id, status: 'PASS', ...await work()})}
  catch (error) {
    const states = await listed().then(rows => rows.map(row => ({command: row.command.at(-1), state: row.state})), () => [])
    cells.push({id, status: 'FAIL', message: error.message, states}); throw error
  }
  finally {await writeFile(path.join(evidence, 'reservation-peaks-results.json'), JSON.stringify(result, null, 2))}
}
await mkdir(evidence, {recursive: true})
try {
  // The kernel delegates workloads beside its own cgroup. Bound their actual
  // parent, so this drill exercises a real small machine budget on the builder.
  await mkdir(cgroup)
  ownsCgroup = true
  result.cgroup = {path: cgroup, limit_bytes: 6 * 1024 * mib, removed: false}
  await writeFile(path.join(cgroup, 'memory.max'), String(6 * 1024 * mib))
  await writeFile(path.join(cgroup, 'memory.swap.max'), '0')
  await writeFile(path.join(cgroup, 'cgroup.subtree_control'), '+memory')
  await fixture.start(); await fixture.grant()
  const cwd = path.join(home, 'A/work')
  session = (await fixture.A.send({CreateSession: {workspace_id: cwd, worktree_id: cwd, alias: 'Binary admission', no_subscribe: true}})).SessionCreated.session.id
  if (mode !== 'peaks') await cell('resident-memory-consumes-launch-reservation', async () => {
    const first = run('reserved-work', 1536); await ready(first)
    await until(async () => {
      const reply = await fixture.A.send({GetSessionState: {session_id: session}})
      return reply.SessionState.processes.reduce((sum, row) => sum + row.resident_set_bytes, 0) >= 1536 * mib
    }, 'resident binary appears in process observations')
    const second = run('reserved-work', 0)
    await ready(second)
    await finish(second); await finish(first)
    return {first_resident_mib: 1536, predicted_mib: 2048, both_started: true}
  })
  if (mode !== 'reservations') await cell('completed-peak-reloads-and-queues-under-reservation-pressure', async () => {
    const trained = run('learned-work', 640); await ready(trained)
    await new Promise(resolve => setTimeout(resolve, 1200))
    await finish(trained)
    const cache = path.join(fixture.envs.A.CHARIOX_HOME, 'state/admission-peaks.json')
    const peaks = JSON.parse(await readFile(cache, 'utf8'))
    const learned = peaks[key(command('learned-work'), cwd)]
    assert.ok(learned?.memory >= 640 * mib, 'completed binary peak must be saved before its final outcome')
    const kernel = fixture.children.find(row => row.name === 'kernel-A').child
    await fixture.A.close(); signal(kernel, 'SIGTERM'); await kernel.done
    await fixture.launch('kernel-A-restarted', fixture.kernelBinary, fixture.envs.A)
    fixture.A = new LocalIpcClient(`ws://127.0.0.1:${fixture.ports.A}/kernel`, {localAuthEnvironment: fixture.envs.A, controlRequestRetryDeadlineMs: 0})
    fixture.clients.push(fixture.A)
    await until(async () => {try {return (await fixture.A.send({RelayStatus: null})).RelayStatus.status.connected} catch {return false}}, 'kernel restarted', 60_000)
    const pressure = run('pressure-work', 0); await ready(pressure)
    const repeat = run('learned-work', 0)
    const queued = await until(async () => (await listed()).find(row => row.command.at(-1) === 'learned-work' && row.state.kind === 'queued'), 'reloaded heavy binary waits under pressure')
    assert.equal(repeat.output, '')
    assert.ok(queued.state.wait.need_bytes >= 640 * mib)
    assert.ok(queued.state.wait.free_bytes > 16 * mib, 'unknown command would fit')
    await finish(pressure)
    await ready(repeat); await finish(repeat)
    return {learned_peak: learned.memory, queued_need: queued.state.wait.need_bytes, free_bytes: queued.state.wait.free_bytes, restart: true, auto_admitted: true}
  })
  result.status = 'PASS'
} finally {
  for (const child of active) signal(child, 'SIGTERM')
  await Promise.allSettled([...active].map(child => child.done))
  await fixture.cleanup()
  const remove = async directory => {
    for (const child of await readdir(directory, {withFileTypes: true})) if (child.isDirectory()) await remove(path.join(directory, child.name))
    await rmdir(directory)
  }
  if (ownsCgroup) {await remove(cgroup); result.cgroup.removed = true}
  await writeFile(path.join(evidence, 'reservation-peaks-results.json'), JSON.stringify(result, null, 2))
}
