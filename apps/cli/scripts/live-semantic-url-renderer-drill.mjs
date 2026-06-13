#!/usr/bin/env node
import { spawn } from 'node:child_process'
import http from 'node:http'
import net from 'node:net'
import path from 'node:path'
import { mkdir, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts } from './lib/drill-artifacts.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
const { createDefaultShellContext, parseShellCommand } = await import('../../../packages/kernel-client/dist/shell-core.js')
const { executeShellCommand } = await import('../../../packages/kernel-client/dist/shell-executor.js')

const {
  endSessionRequest,
  getProviderRunRequest,
  getWorkflowRunRequest,
  launchProviderRunRequest,
  listSessionsRequest,
  pumpTerminalOutputRequest,
  updateWorkflowNodeInstructionsRequest,
} = requests

function logStep(name, details = null) {
  if (details == null) console.log(`[semantic-url-renderer-drill] ${name}`)
  else console.log(`[semantic-url-renderer-drill] ${name}`, JSON.stringify(details))
}

function nowStamp() {
  return new Date().toISOString().replace(/[-:]/g, '').replace(/\.\d{3}Z$/, 'Z')
}

function variant(response, key) {
  return response?.[key] ?? response
}

async function run(command, args, options = {}) {
  return await new Promise((resolve) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', (error) => resolve({ code: 1, stdout, stderr: String(error) }))
    child.on('close', (code) => resolve({ code, stdout, stderr }))
  })
}

function startProcess(command, args, env, name) {
  const logs = { stdout: '', stderr: '' }
  const child = spawn(command, args, {
    cwd: repoRoot,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  child.stdout.on('data', (chunk) => { logs.stdout += chunk.toString() })
  child.stderr.on('data', (chunk) => { logs.stderr += chunk.toString() })
  child.logs = logs
  child.name = name
  return child
}

async function stopProcess(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  await new Promise((resolve) => {
    const timeout = setTimeout(() => {
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
    }, 3_000)
    child.once('exit', () => {
      clearTimeout(timeout)
      resolve()
    })
    child.kill('SIGTERM')
  })
}

async function cleanupListeningPorts(ports) {
  for (const port of ports) {
    const result = await run('lsof', ['-ti', `tcp:${port}`]).catch(() => ({ code: 1, stdout: '' }))
    if (result.code !== 0 || !result.stdout.trim()) continue
    const pids = result.stdout
      .split(/\s+/)
      .map((value) => Number(value))
      .filter((pid) => Number.isInteger(pid) && pid > 0 && pid !== process.pid)
    for (const pid of pids) {
      try {
        process.kill(pid, 'SIGTERM')
      } catch {}
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
    for (const pid of pids) {
      try {
        process.kill(pid, 0)
        process.kill(pid, 'SIGKILL')
      } catch {}
    }
  }
}

async function freePort() {
  return await new Promise((resolve, reject) => {
    const server = net.createServer()
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      const port = typeof address === 'object' && address ? address.port : null
      server.close(() => port ? resolve(port) : reject(new Error('could not allocate port')))
    })
    server.on('error', reject)
  })
}

async function buildKernel() {
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
}

async function waitForKernel(kernelUrl) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(listSessionsRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }
  throw new Error(`kernel did not become ready: ${lastError?.message ?? String(lastError)}`)
}

async function waitForGateway(baseUrl) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const response = await fetchWithTimeout(`${baseUrl}/health`, {}, 5_000)
      if (response.ok) return
      lastError = new Error(`health status ${response.status}`)
    } catch (error) {
      lastError = error
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`gateway did not become ready: ${lastError?.message ?? String(lastError)}`)
}

async function fetchWithTimeout(url, options = {}, timeoutMs = 15_000) {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(new Error(`fetch timed out after ${timeoutMs}ms: ${url}`)), timeoutMs)
  try {
    return await fetch(url, { ...options, signal: controller.signal })
  } finally {
    clearTimeout(timer)
  }
}

async function withTimeout(promise, timeoutMs, message) {
  let timer = null
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(message)), timeoutMs)
      }),
    ])
  } finally {
    if (timer) clearTimeout(timer)
  }
}

async function waitForProviderRunReady(client, providerRunId) {
  const deadline = Date.now() + 45_000
  while (Date.now() < deadline) {
    const providerRun = variant(await client.send(getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run
    if (providerRun?.state && providerRun.state !== 'Starting') {
      if (providerRun.state !== 'Running' && providerRun.state !== 'Parked') {
        throw new Error(`provider run ${providerRunId} reached unexpected state ${providerRun.state}`)
      }
      return providerRun
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`provider run ${providerRunId} did not become ready`)
}

async function startStaticSite(port) {
  const pages = new Map([
    ['/about', '<!doctype html><html><head><title>About Arroba Foods</title></head><body><main><h1>About Arroba Foods</h1><p>We build practical grocery tools for neighborhood stores.</p><a href="/contact">Contact us</a></main></body></html>'],
    ['/contact', '<!doctype html><html><head><title>Contact Arroba Foods</title></head><body><main><h1>Contact</h1><p>Email hello@arroba-foods.example for store onboarding.</p><a href="/about">About</a></main></body></html>'],
    ['/pricing', '<!doctype html><html><head><title>Pricing</title></head><body><main><h1>Pricing</h1><p>Starter, Market, and Fleet plans are available.</p></main></body></html>'],
  ])
  const server = http.createServer((req, res) => {
    const url = new URL(req.url ?? '/', `http://${req.headers.host ?? `127.0.0.1:${port}`}`)
    const body = pages.get(url.pathname)
    if (!body) {
      res.writeHead(404, { 'content-type': 'text/plain' })
      res.end('not found')
      return
    }
    res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
    res.end(body)
  })
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(port, '127.0.0.1', resolve)
  })
  return server
}

async function closeHttpServer(server) {
  if (!server) return
  server.closeAllConnections?.()
  server.closeIdleConnections?.()
  await new Promise((resolve) => server.close(resolve)).catch(() => {})
}

async function startSemanticSite({ port, gatewayUrl, kernelUrl, sessionId }) {
  const cache = new Map()
  const client = new LocalIpcClient(kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  const server = http.createServer(async (req, res) => {
    try {
      const url = new URL(req.url ?? '/', `http://${req.headers.host ?? `127.0.0.1:${port}`}`)
      const [page, ...promptParts] = url.pathname.split('/').filter(Boolean)
      if (!page || promptParts.length === 0) {
        res.writeHead(404, { 'content-type': 'text/plain' })
        res.end('semantic render routes are /<page>/<prompt>')
        return
      }
      const prompt = decodeURIComponent(promptParts.join('/'))
      const key = `${page}:${prompt}`
      let runId = cache.get(key)
      if (!runId) {
        const gatewayResponse = await fetchWithTimeout(`${gatewayUrl}/render/${encodeURIComponent(page)}/${encodeURIComponent(prompt)}`)
        const body = await gatewayResponse.json()
        if (gatewayResponse.status !== 202 || !body.workflow_run?.id) {
          res.writeHead(502, { 'content-type': 'application/json' })
          res.end(JSON.stringify({ error: 'publication did not accept render', body }))
          return
        }
        runId = body.workflow_run.id
        cache.set(key, runId)
      }

      const run = variant(await client.send(getWorkflowRunRequest(sessionId, runId)), 'WorkflowRun').workflow_run
      if (run.status !== 'Completed') {
        res.writeHead(202, { 'content-type': 'text/html; charset=utf-8' })
        res.end(`<!doctype html><html><head><meta http-equiv="refresh" content="1"><title>Loading render</title></head><body><main><h1>Loading...</h1><p>Rendering ${page} with Arroba workflow run ${runId}</p><p>Status: ${run.status}</p></main></body></html>`)
        return
      }
      const message = run.final_output?.message ?? ''
      let html = message
      try {
        const parsed = JSON.parse(message)
        if (parsed.kind === 'http_response') {
          html = typeof parsed.body === 'string' ? parsed.body : JSON.stringify(parsed.body)
        }
      } catch {
        html = message
      }
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
      res.end(html)
    } catch (error) {
      res.writeHead(500, { 'content-type': 'text/plain' })
      res.end(error.stack ?? error.message)
    }
  })
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(port, '127.0.0.1', resolve)
  })
  return {
    async close() {
      await withTimeout(
        client.close(),
        3_000,
        'semantic site client close timed out after 3000ms',
      ).catch(() => {})
      await closeHttpServer(server)
    },
  }
}

async function fetchUntilRendered(url, options = {}) {
  const timeoutMs = options.timeoutMs ?? 420_000
  const deadline = Date.now() + timeoutMs
  let firstBody = null
  let lastBody = null
  const diagnostics = []
  while (Date.now() < deadline) {
    if (options.pump) {
      try {
        const diagnostic = await withTimeout(
          Promise.resolve(options.pump()),
          options.pumpTimeoutMs ?? 5_000,
          `terminal pump timed out after ${options.pumpTimeoutMs ?? 5_000}ms`,
        )
        if (diagnostic) diagnostics.push(diagnostic)
      } catch (error) {
        diagnostics.push({ pump_error: error instanceof Error ? error.message : String(error) })
      }
    }
    let response = null
    let body = ''
    try {
      response = await fetchWithTimeout(url, {}, options.fetchTimeoutMs ?? 15_000)
      body = await response.text()
    } catch (error) {
      body = error instanceof Error ? error.message : String(error)
      if (!firstBody) firstBody = { status: 0, body }
      lastBody = { status: 0, body }
      diagnostics.push({ fetch_error: body })
      await new Promise((resolve) => setTimeout(resolve, 2_000))
      continue
    }
    if (!firstBody) firstBody = { status: response.status, body }
    lastBody = { status: response.status, body }
    if (response.status === 200 && body.includes('ARROBA_RENDER_NEON_GREEN') && body.includes('background') && body.includes('About Arroba Foods')) {
      return { firstBody, finalBody: lastBody }
    }
    await new Promise((resolve) => setTimeout(resolve, 2_000))
  }
  throw new Error(`semantic render did not complete\nfirst=${JSON.stringify(firstBody)}\nlast=${JSON.stringify(lastBody)}\ndiagnostics=${JSON.stringify(diagnostics.slice(-10), null, 2)}`)
}

async function main() {
  const provider = process.env.ARROBA_SEMANTIC_RENDER_PROVIDER ?? 'codex'
  const model = process.env.ARROBA_SEMANTIC_RENDER_MODEL ?? 'gpt-5.4'
  const effort = process.env.ARROBA_SEMANTIC_RENDER_EFFORT ?? 'low'
  const root = path.join(repoRoot, '.artifacts', 'semantic-url-renderer', nowStamp())
  const workspace = path.join(root, 'workspace')
  const home = path.join(root, 'home')
  const configHome = path.join(root, 'config')
  const stateHome = path.join(root, 'state')
  const kernelPort = await freePort()
  const mcpPort = await freePort()
  const opencodePort = await freePort()
  const codexPort = await freePort()
  const staticPort = await freePort()
  const gatewayPort = await freePort()
  const semanticPort = await freePort()
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const gatewayUrl = `http://127.0.0.1:${gatewayPort}`
  const staticUrl = `http://127.0.0.1:${staticPort}`
  const semanticUrl = `http://127.0.0.1:${semanticPort}`
  const env = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: stateHome,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(opencodePort),
    ARROBA_CODEX_PORT: String(codexPort),
    ARROBA_DAEMON_ID: `semantic-url-renderer-drill-${process.pid}`,
    ARROBA_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(root, 'history'),
  }

  let kernel = null
  let gateway = null
  let staticSite = null
  let semanticSite = null
  let client = null
  let sessionId = null
  let succeeded = false
  let failure = null
  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })
    await writeFile(path.join(configHome, 'arroba', 'config.toml'), 'version = 1\n', 'utf8')

    const parserFile = path.join(root, 'semantic-parser.mjs')
    await writeFile(parserFile, [
      "let input = '';",
      "process.stdin.on('data', (chunk) => { input += chunk.toString() });",
      "process.stdin.on('end', async () => {",
      "  const req = JSON.parse(input || '{}');",
      "  const parts = String(req.url || '').split('?')[0].split('/').filter(Boolean);",
      "  const page = decodeURIComponent(parts[1] || '');",
      "  const prompt = decodeURIComponent(parts.slice(2).join('/') || '');",
      "  const source_url = `${process.env.STATIC_SITE_BASE_URL}/${page}`;",
      "  const source_html = await fetch(source_url).then((response) => response.text());",
      "  process.stdout.write(JSON.stringify({ page, prompt, source_url, source_html }));",
      "});",
    ].join('\n'), 'utf8')

    staticSite = await startStaticSite(staticPort)
    logStep('static_site_ready', { staticUrl })

    const kernelBinary = await buildKernel()
    kernel = startProcess(kernelBinary, [], env, 'kernel')
    await waitForKernel(kernelUrl)
    client = new LocalIpcClient(kernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })

    const context = createDefaultShellContext({
      workspace,
      worktree: workspace,
      provider,
      model,
      effort,
    })
    const runShell = async (command) => {
      logStep('shell', { command })
      const result = await executeShellCommand(parseShellCommand(command, context), context, {
        client,
        clientId: `semantic-url-renderer-drill-${process.pid}`,
      })
      if (!result.ok) throw new Error(`shell command failed: ${command}\n${result.message}`)
      Object.assign(context, result.contextUpdates ?? {})
      Object.assign(context.variables, result.bindings ?? {})
      return result
    }

    await runShell(`session new ${workspace} as session`)
    sessionId = context.sessionId
    const agentResult = await runShell(`agent spawn renderer ${model} --dir ${workspace} as renderer`)
    const agent = agentResult.data.agent

    const workflowResult = await runShell('workflow new semantic-url-renderer as workflow')
    const workflow = workflowResult.data.workflow
    const nodeResult = await runShell('workflow node add $workflow $renderer as node')
    const node = nodeResult.data.node
    await runShell('workflow node can-complete-run $workflow $node true')
    await runShell('workflow node max-turns $workflow $node 3')
    await runShell('workflow run-output-schema $workflow none')
    await client.send(updateWorkflowNodeInstructionsRequest(
      sessionId,
      workflow.id,
      node.id,
      [
        'You render a static source HTML page according to a URL prompt.',
        'The workflow invocation input is JSON with page, prompt, source_url, and source_html.',
        'Generate a complete standalone HTML document.',
        'Preserve the source page semantic content, especially headings and contact/about/pricing facts.',
        'Apply the requested visual style strongly.',
        'For green neon on black, use a black background, neon green accents, and include the exact marker text ARROBA_RENDER_NEON_GREEN in a data attribute or hidden text.',
        'Submit final workflow run output. The output message must be JSON string content shaped exactly as {"kind":"http_response","status":200,"headers":{"content-type":"text/html; charset=utf-8"},"body":"<full html document>"}',
        'Also emit a final fenced json workflow output block whose output.message is that same http_response JSON string.',
      ].join('\n'),
    ))
    const endpointResult = await runShell('workflow endpoint new $workflow $node render-endpoint as endpoint')
    const endpoint = endpointResult.data.endpoint
    context.variables.endpoint = endpoint.id
    const parserJson = JSON.stringify({ kind: 'custom_command', command: process.execPath, args: [parserFile] })
    const inputSchemaJson = JSON.stringify({
      type: 'object',
      required: ['page', 'prompt', 'source_url', 'source_html'],
      properties: {
        page: { type: 'string' },
        prompt: { type: 'string' },
        source_url: { type: 'string' },
        source_html: { type: 'string' },
      },
    })
    const publicationResult = await runShell(`workflow publication create $workflow $endpoint semantic-render --route /render/* --method GET --parser-json '${parserJson}' --input-schema-json '${inputSchemaJson}' --mode async`)
    const publication = publicationResult.data.publication

    logStep('start_gateway', { publicationId: publication.id })
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        STATIC_SITE_BASE_URL: staticUrl,
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: sessionId,
        ARROBA_PUBLICATION_ID: publication.id,
      },
      'gateway',
    )
    await waitForGateway(gatewayUrl)

    semanticSite = await startSemanticSite({ port: semanticPort, gatewayUrl, kernelUrl, sessionId })
    logStep('semantic_site_ready', { semanticUrl })

    const renderUrl = `${semanticUrl}/about/${encodeURIComponent('serve me this page with green neon colors in black background')}`
    logStep('invoke_semantic_url', { renderUrl })
    const render = await fetchUntilRendered(renderUrl, {
      timeoutMs: Number(process.env.ARROBA_SEMANTIC_RENDER_TIMEOUT_MS ?? 420_000),
      fetchTimeoutMs: Number(process.env.ARROBA_SEMANTIC_RENDER_FETCH_TIMEOUT_MS ?? 15_000),
      pumpTimeoutMs: Number(process.env.ARROBA_SEMANTIC_RENDER_PUMP_TIMEOUT_MS ?? 5_000),
      pump: async () => {
        if (context.attachmentId) {
          const response = await client.send(pumpTerminalOutputRequest(sessionId, context.attachmentId)).catch(() => null)
          const records = variant(response, 'TerminalOutput')?.records ?? []
          const text = records.map((record) => Array.isArray(record.bytes) ? Buffer.from(record.bytes).toString('utf8') : '').join('')
          if (text.trim()) return { text: text.slice(-1200) }
        }
        return null
      },
    })
    if (render.firstBody.status !== 202 || !render.firstBody.body.includes('Loading...')) {
      throw new Error(`expected first semantic response to be a loading page, got ${render.firstBody.status}: ${render.firstBody.body.slice(0, 400)}`)
    }
    if (!render.finalBody.body.includes('ARROBA_RENDER_NEON_GREEN') || !render.finalBody.body.includes('About Arroba Foods')) {
      throw new Error(`rendered page missing expected marker/content: ${render.finalBody.body.slice(0, 800)}`)
    }
    logStep('render_ok', {
      initialStatus: render.firstBody.status,
      finalStatus: render.finalBody.status,
      finalBytes: render.finalBody.body.length,
    })

    logStep('ok')
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (semanticSite?.close) {
      await withTimeout(
        semanticSite.close(),
        5_000,
        'semantic site close timed out after 5000ms',
      ).catch((error) => logStep('semantic_site_close_failed', { error: error.message ?? String(error) }))
    }
    await closeHttpServer(staticSite)
    if (client && sessionId) {
      await withTimeout(
        client.send(endSessionRequest(sessionId)),
        5_000,
        'end session timed out after 5000ms',
      ).catch((error) => logStep('end_session_failed', { error: error.message ?? String(error) }))
    }
    if (client?.close) {
      await withTimeout(
        client.close(),
        3_000,
        'client close timed out after 3000ms',
      ).catch((error) => logStep('client_close_failed', { error: error.message ?? String(error) }))
    }
    await stopProcess(gateway)
    await stopProcess(kernel)
    await cleanupListeningPorts([opencodePort, codexPort])
    if (!succeeded) {
      console.error('[semantic-url-renderer-drill] kernel logs', kernel?.logs ?? null)
      console.error('[semantic-url-renderer-drill] gateway logs', gateway?.logs ?? null)
    }
    await finalizeDrillArtifacts({
      rootDir: root,
      passed: succeeded,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: 'semantic-url-renderer',
        provider,
        model,
        effort,
        workspace,
        kernelUrl,
        gatewayUrl,
        staticUrl,
        semanticUrl,
        kernelStdoutTail: kernel?.logs?.stdout?.slice(-4000) ?? '',
        kernelStderrTail: kernel?.logs?.stderr?.slice(-4000) ?? '',
        gatewayStdoutTail: gateway?.logs?.stdout?.slice(-4000) ?? '',
        gatewayStderrTail: gateway?.logs?.stderr?.slice(-4000) ?? '',
      },
      log: logStep,
    })
  }
}

main().catch((error) => {
  console.error(`[semantic-url-renderer-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
