#!/usr/bin/env node
// MP-08 / MP-09 / MP-10 / MP-11: offline check of Claude's root YOLO compatibility.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdtemp, mkdir, readFile, rm } from 'node:fs/promises'
import http from 'node:http'
import { homedir } from 'node:os'
import { join } from 'node:path'

assert.equal(process.geteuid?.(), 0, 'Run this drill as root; use a disposable kernel host.')
const claude = process.argv[2] ?? 'claude'
const stateRoot = process.env.CHARIOX_HOME ?? join(homedir(), '.chariox/dev/claude-root-yolo-drill')
await mkdir(stateRoot, { recursive: true, mode: 0o700 })
const scratch = await mkdtemp(join(stateRoot, 'run-'))
const refusal = '--dangerously-skip-permissions cannot be used with root/sudo privileges'
let requests = 0
let proof

const server = http.createServer(async (request, response) => {
  let body = ''
  for await (const chunk of request) body += chunk
  if (request.method !== 'POST' || !request.url.startsWith('/v1/messages')) {
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end('{}')
    return
  }
  requests++
  const input = JSON.parse(body)
  const toolResult = input.messages?.some(message => Array.isArray(message.content)
    && message.content.some(block => block.type === 'tool_result'))
  const useTool = input.tools?.some(tool => tool.name === 'Bash') && !toolResult
  const block = useTool
    ? { type: 'tool_use', id: 'toolu_root_yolo', name: 'Bash', input: {} }
    : { type: 'text', text: 'Root YOLO drill complete.' }
  const toolInput = JSON.stringify({ command: `id -u > '${proof}'`, description: 'Record drill uid' })
  const stopReason = useTool ? 'tool_use' : 'end_turn'
  const message = {
    id: 'msg_root_yolo', type: 'message', role: 'assistant', model: input.model,
    content: [], stop_reason: null, usage: { input_tokens: 10, output_tokens: 5 },
  }
  if (!input.stream) {
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end(JSON.stringify({ ...message, content: [useTool ? { ...block, input: JSON.parse(toolInput) } : block], stop_reason: stopReason }))
    return
  }
  response.writeHead(200, { 'content-type': 'text/event-stream' })
  const event = (type, fields = {}) => response.write(`event: ${type}\ndata: ${JSON.stringify({ type, ...fields })}\n\n`)
  event('message_start', { message })
  event('content_block_start', { index: 0, content_block: block })
  event('content_block_delta', { index: 0, delta: useTool
    ? { type: 'input_json_delta', partial_json: toolInput }
    : { type: 'text_delta', text: block.text } })
  event('content_block_stop', { index: 0 })
  event('message_delta', { delta: { stop_reason: stopReason, stop_sequence: null }, usage: { output_tokens: 5 } })
  event('message_stop')
  response.end()
})

function run(args, env, cwd) {
  return new Promise((resolve, reject) => {
    const child = spawn(claude, args, { env, cwd, stdio: ['ignore', 'pipe', 'pipe'], timeout: 60_000 })
    let output = ''
    for (const stream of [child.stdout, child.stderr]) {
      stream.on('data', chunk => { output = (output + chunk).slice(-64_000) })
    }
    child.once('error', reject)
    child.once('close', (code, signal) => resolve({ code, signal, output }))
  })
}

try {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  for (const shim of [false, true]) {
    const directory = join(scratch, shim ? 'shim' : 'control')
    await mkdir(directory, { mode: 0o700 })
    proof = join(directory, 'proof.txt')
    // An allowlist prevents inherited provider credentials, settings and sandbox overrides.
    const env = {
      PATH: process.env.PATH, HOME: directory, CLAUDE_CONFIG_DIR: join(directory, 'config'),
      DISABLE_AUTOUPDATER: '1', CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
      ANTHROPIC_API_KEY: 'sk-ant-FAKE-offline-drill',
      ANTHROPIC_BASE_URL: `http://127.0.0.1:${server.address().port}`,
      ...(shim ? { IS_SANDBOX: '1' } : {}),
    }
    if (!shim) {
      const version = await run(['--version'], env, directory)
      assert.equal(version.code, 0, 'Claude --version failed')
      console.log(`Claude: ${version.output.trim()}`)
    }
    requests = 0
    const result = await run([
      '-p', 'Run the requested Bash tool once.', '--model', 'claude-haiku-5-5',
      '--output-format', 'json', '--permission-mode', 'bypassPermissions',
      '--allow-dangerously-skip-permissions',
    ], env, directory)
    assert.equal(result.signal, null, 'Claude timed out or was signalled')
    if (!shim) {
      assert.notEqual(result.code, 0, 'Control unexpectedly accepted root YOLO; re-evaluate the compatibility shim')
      assert.ok(result.output.includes(refusal), 'Control did not report the recorded root refusal')
      assert.equal(requests, 0, 'Refused control unexpectedly contacted the fake API')
      console.log('PASS control: root YOLO refused before any API request')
    } else {
      assert.equal(result.code, 0, 'IS_SANDBOX=1 no longer permits root YOLO; update Chariox before bumping Claude')
      assert.equal((await readFile(proof, 'utf8')).trim(), '0', 'Root Bash tool did not run without a permission prompt')
      assert.ok(requests >= 2, 'Missing tool-result continuation')
      console.log('PASS shim: root Bash tool ran without a permission prompt and returned its result')
    }
  }
} finally {
  await new Promise(resolve => server.close(resolve))
  await rm(scratch, { recursive: true, force: true })
}
