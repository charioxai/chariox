#!/usr/bin/env node
// MP-08 / MP-10: a tool result with images larger than the 16 KB record cap reaches the live stream and the
// persisted history whole, and an image past the kernel's budget is counted in `images_omitted`.
// Real kernel binary, real local client; no provider run is needed (a dev-stub run only supplies the run id).
//   node scripts/drills/tool-result-images.mjs --binary=<kernel> --root=<scratch dir> [--client-dist=<kernel-client dist>]
import assert from 'node:assert/strict'
import { spawn, execFileSync } from 'node:child_process'
import { createWriteStream } from 'node:fs'
import { mkdir, writeFile, readFile } from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { createHash } from 'node:crypto'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const opts = Object.fromEntries(process.argv.slice(2).map(value => {
  const [key, ...rest] = value.replace(/^--/, '').split('=')
  return [key, rest.length ? rest.join('=') : true]
}))
assert.equal(typeof opts.binary, 'string', '--binary=<built kernel> is required')
assert.equal(typeof opts.root, 'string', '--root=<scratch directory outside the repository> is required')
assert.ok(!path.resolve(opts.root).startsWith(repo), 'state must live outside the repository')
const api = await import(pathToFileURL(path.resolve(opts['client-dist'] ?? path.join(repo, 'packages/kernel-client/dist'), 'index.js')).href)
const { isolatedKernelConfigToml } = await import(pathToFileURL(path.join(repo, 'apps/cli/scripts/lib/drill-kernel-storage.mjs')).href)

const runRoot = path.join(path.resolve(opts.root), `tool-images-${Date.now()}`)
const stateRoot = path.join(runRoot, 'home')
const workspace = path.join(runRoot, 'work')
await mkdir(stateRoot, { recursive: true, mode: 0o700 })
await mkdir(workspace, { recursive: true })
execFileSync('git', ['init', '-q', workspace])
await writeFile(path.join(workspace, 'README.md'), '# tool result images drill\n')
const results = { area: 'MP-08 / MP-10', checks: [], binary_sha256: createHash('sha256').update(await readFile(opts.binary)).digest('hex') }
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
const check = (name, condition) => { assert.ok(condition, name); results.checks.push(name); console.log(JSON.stringify({ check: name, status: 'PASS' })) }
const unwrap = (response, key) => { if (!response || typeof response !== 'object' || !(key in response)) assert.fail(`expected ${key}, got ${Object.keys(response ?? {})}`); return response[key] }
const port = async () => { const server = net.createServer(); await new Promise(resolve => server.listen(0, '127.0.0.1', resolve)); const result = server.address().port; await new Promise(resolve => server.close(resolve)); return result }
const kernelPort = await port()
const inherited = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('CHARIOX_')))
const env = { ...inherited, HOME: path.join(stateRoot, 'home'), XDG_CONFIG_HOME: path.join(stateRoot, 'xdg-config'), XDG_STATE_HOME: path.join(stateRoot, 'xdg-state'), XDG_CACHE_HOME: path.join(stateRoot, 'xdg-cache'), XDG_DATA_HOME: path.join(stateRoot, 'xdg-data'), CHARIOX_HOME: stateRoot, CHARIOX_KERNEL_HOST: '127.0.0.1', CHARIOX_KERNEL_PORT: String(kernelPort), CHARIOX_MCP_PORT: String(await port()), CHARIOX_CODEX_PORT: String(await port()), CHARIOX_OPENCODE_PORT: String(await port()), CHARIOX_DAEMON_SOCKET: path.join(runRoot, 'kernel.sock'), CHARIOX_DAEMON_ID: `tool-images-${Date.now()}`, CHARIOX_SESSION_HISTORY_DIR: path.join(stateRoot, 'history'), CHARIOX_KERNEL_RUNTIME_ROLE: 'general' }
await mkdir(env.HOME, { recursive: true })
await writeFile(path.join(stateRoot, 'config.toml'), isolatedKernelConfigToml(path.join(stateRoot, 'storage')), { mode: 0o600 })

let child, client
async function stop() {
  await client?.close().catch(() => {})
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  assert.ok(Number.isSafeInteger(child.pid) && child.pid > 1, 'refuse invalid child PID')
  child.kill('SIGTERM')
  await Promise.race([new Promise(resolve => child.once('exit', resolve)), pause(8000)])
  if (child.exitCode === null && child.signalCode === null) {
    assert.ok(Number.isSafeInteger(child.pid) && child.pid > 1, 'refuse invalid child PID')
    child.kill('SIGKILL')
    await new Promise(resolve => child.once('exit', resolve))
  }
}
try {
  const log = createWriteStream(path.join(runRoot, 'kernel.log'), { mode: 0o600 })
  child = spawn(opts.binary, [], { cwd: workspace, env, stdio: ['ignore', 'pipe', 'pipe'] })
  child.stdout.pipe(log, { end: false }); child.stderr.pipe(log, { end: false })
  for (let attempt = 0; ; attempt++) {
    assert.ok(attempt < 120 && child.exitCode === null, 'kernel did not become ready')
    client = new api.LocalIpcClient(`ws://127.0.0.1:${kernelPort}/kernel`, { localAuthEnvironment: env })
    try { await client.send(api.listSessionsRequest()); break } catch { await client.close().catch(() => {}); await pause(250) }
  }
  check('kernel ready', true)
  const created = unwrap(await client.send(api.createSessionRequest(workspace, workspace, 'tool-images', { provider: 'dev-stub', model: 'default', effort: 'default' })), 'SessionCreated')
  const room = created.session.id, agent = created.agent.id
  const attachment = unwrap(await client.send(api.attachToSessionRequest(room, 'tool-images-drill')), 'SessionAttached').attachment.id
  await client.send(api.submitPromptRequest(room, attachment, agent, 'hello', []))
  let runId
  for (let attempt = 0; !runId; attempt++) {
    assert.ok(attempt < 200, 'no provider run appeared')
    const { records } = unwrap(await client.send(api.pumpTerminalOutputRequest(room, attachment)), 'TerminalOutput')
    runId = records.find(record => record.provider_run_id)?.provider_run_id
    if (!runId) await pause(100)
  }
  check('a provider run exists to attach the tool result to', true)

  const image = `data:image/png;base64,${'iVBORw0KGgo='.repeat(8 * 1024)}`
  const tooBig = `data:image/png;base64,${'A'.repeat(3 * 1024 * 1024)}`
  assert.ok(image.length > 64 * 1024)
  const payload = { id: 'shot', tool: 'Read', status: 'completed', input: { file_path: '/work/chart.png' }, output: 'chart.png', images: [image, image, tooBig] }
  unwrap(await client.send(api.appendNativeProviderOutputRequest(room, attachment, runId, 'provider_tool', JSON.stringify(payload), 'shot')), 'TerminalOutput')

  const wholeImages = value => {
    assert.deepEqual(value.images, [image, image], 'the images over the 16 KB cap arrive whole')
    assert.equal(value.images_omitted, 1, 'the image past the budget is counted')
    assert.equal(value.output, 'chart.png')
  }
  // Live: the next pump replays what the stream carried for this room.
  const live = []
  for (let attempt = 0; attempt < 50 && live.length === 0; attempt++) {
    const { records } = unwrap(await client.send(api.pumpTerminalOutputRequest(room, attachment)), 'TerminalOutput')
    live.push(...records.filter(record => record.kind === 'provider_tool' && record.merge_key === 'shot'))
    if (live.length === 0) await pause(100)
  }
  check('the live stream carried the tool result', live.length > 0)
  wholeImages(JSON.parse(Buffer.from(live.at(-1).bytes).toString('utf8')))
  check('live record: images whole, omission counted', true)

  // Persisted: the history blob the web opens.
  const outline = unwrap(await client.send(api.getSessionHistoryOutlineRequest(room, [agent], 5, null, true)), 'SessionHistoryOutline')
  const blobs = outline.agents.flatMap(entry => entry.turns).flatMap(turn => turn.blobs ?? []).filter(blob => blob.kind === 'provider_tool')
  check('history holds the tool result as a blob', blobs.length === 1)
  check('the outline preview carries no image bytes', !blobs[0].preview?.includes('base64') && !blobs[0].preview?.includes('images"'))
  const content = unwrap(await client.send(api.getSessionHistoryBlobContentRequest(room, agent, blobs[0].blob_id)), 'SessionHistoryBlobContent')
  const stored = content.entries.map(row => row.entry?.text ?? row.text).filter(Boolean).map(text => JSON.parse(text)).at(-1)
  wholeImages(stored)
  check('persisted history: images whole, omission counted', true)
  results.status = 'PASS'
} catch (error) {
  results.status = 'FAIL'
  results.error = String(error?.message ?? error).slice(0, 2000)
  console.error(results.error)
  process.exitCode = 1
} finally {
  await stop()
  await writeFile(path.join(runRoot, 'result.json'), JSON.stringify(results, null, 2) + '\n', { mode: 0o600 })
  console.log(JSON.stringify({ result: results.status, evidence: path.join(runRoot, 'result.json') }))
}
