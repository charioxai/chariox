import { spawn } from 'node:child_process'
import net from 'node:net'
import path from 'node:path'
import { cp, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { WebSocket } from 'ws'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
const { createDefaultShellContext, parseShellCommand } = await import('../../../packages/kernel-client/dist/shell-core.js')
const { executeShellCommand } = await import('../../../packages/kernel-client/dist/shell-executor.js')

const {
  addWorkflowNodeRequest,
  attachToSessionRequest,
  createSessionRequest,
  createWorkflowEndpointRequest,
  createWorkflowPublicationRequest,
  createWorkflowRequest,
  createWorkflowWatchdogRequest,
  endSessionRequest,
  getDaemonHealthRequest,
  getSessionStateRequest,
  getWorkflowPublicationRequest,
  getWorkflowRunRequest,
  getProviderRunRequest,
  launchProviderRunRequest,
  listSessionsRequest,
  listWorkflowRunsRequest,
  setWorkflowNodeCanCompleteRunRequest,
  setWorkflowNodeCanEmitIntermediateOutputRequest,
  spawnAgentRequest,
  updateWorkflowNodeInstructionsRequest,
} = requests

function logStep(name, details = null) {
  if (details == null) console.log(`[publication-drill] ${name}`)
  else console.log(`[publication-drill] ${name}`, JSON.stringify(details))
}

function variant(response, key) {
  return response?.[key] ?? response
}

function hasAcceptedRunMetadata(body) {
  return !!body && (body.workflow_run?.id || body.queued === true)
}

function sseEventNames(body) {
  return [...body.matchAll(/^event: (.+)$/gm)].map((match) => match[1])
}

function createWebSocketReader(socket) {
  const queue = []
  const waiters = []
  let socketError = null
  socket.on('message', (data) => {
    let parsed
    try {
      parsed = JSON.parse(data.toString())
    } catch (error) {
      socketError = error
      return
    }
    const waiter = waiters.shift()
    if (waiter) waiter(parsed)
    else queue.push(parsed)
  })
  socket.on('error', (error) => {
    socketError = error
  })
  return {
    read: async () => {
      if (socketError) throw socketError
      const queued = queue.shift()
      if (queued !== undefined) return queued
      return await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error('timed out waiting for websocket message')), 20_000)
        waiters.push((value) => {
          clearTimeout(timeout)
          resolve(value)
        })
      })
    },
  }
}

async function invokePublicationWebSocket(url, input, options = {}) {
  const socket = new WebSocket(url, options)
  const reader = createWebSocketReader(socket)
  try {
    const ready = await reader.read()
    if (ready.type !== 'ready') {
      throw new Error(`expected websocket ready message, got ${JSON.stringify(ready)}`)
    }
    socket.send(JSON.stringify({
      type: 'artifact_begin',
      artifact_id: 'ws-artifact-1',
      name: 'ws-input.txt',
      mime_type: 'text/plain',
      size_bytes: 9,
    }))
    const begun = await reader.read()
    if (begun.type !== 'artifact_ack' || begun.status !== 'begun') {
      throw new Error(`expected websocket artifact begin ack, got ${JSON.stringify(begun)}`)
    }
    socket.send(JSON.stringify({ type: 'artifact_chunk', artifact_id: 'ws-artifact-1', data: 'd3MtcHVibA==' }))
    const chunk = await reader.read()
    if (chunk.type !== 'artifact_ack' || chunk.status !== 'chunk') {
      throw new Error(`expected websocket artifact chunk ack, got ${JSON.stringify(chunk)}`)
    }
    socket.send(JSON.stringify({ type: 'artifact_end', artifact_id: 'ws-artifact-1' }))
    const readyArtifact = await reader.read()
    if (readyArtifact.type !== 'artifact' || readyArtifact.status !== 'ready') {
      throw new Error(`expected websocket artifact ready message, got ${JSON.stringify(readyArtifact)}`)
    }
    socket.send(JSON.stringify({ type: 'invoke', input }))
    const accepted = await reader.read()
    if (accepted.type !== 'accepted' || (!accepted.workflow_run?.id && !accepted.queued)) {
      throw new Error(`expected websocket accepted run metadata, got ${JSON.stringify(accepted)}`)
    }
    if (!options.waitForFinal) {
      return { accepted, messages: [accepted] }
    }
    const messages = [accepted]
    const deadline = Date.now() + (options.timeoutMs ?? 30_000)
    while (Date.now() < deadline) {
      const message = await reader.read()
      messages.push(message)
      if (message.type === 'final' || message.type === 'error' || message.type === 'timeout') {
        return { accepted, messages }
      }
    }
    throw new Error(`timed out waiting for websocket final message: ${JSON.stringify(messages)}`)
  } finally {
    socket.close()
  }
}

function websocketUrlFromHttp(url) {
  const parsed = new URL(url)
  parsed.protocol = parsed.protocol === 'https:' ? 'wss:' : 'ws:'
  return parsed.toString()
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

async function runChecked(command, args, options = {}) {
  const result = await run(command, args, options)
  if (result.code !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return result
}

async function ensureDockerAvailable() {
  await runChecked('docker', ['version', '--format', '{{.Server.Version}}'])
}

async function buildRustBinary(binaryName) {
  const manifestPath = binaryName === 'arroba-relay'
    ? path.join(repoRoot, 'apps/relay/Cargo.toml')
    : path.join(repoRoot, 'apps/kernel/Cargo.toml')
  const result = await run('cargo', ['build', '--manifest-path', manifestPath, '--bin', binaryName])
  if (result.code !== 0) {
    throw new Error(`${binaryName} build failed\n${result.stdout}\n${result.stderr}`)
  }
  const targetRoot = binaryName === 'arroba-relay' ? 'apps/relay' : 'apps/kernel'
  return path.join(repoRoot, targetRoot, 'target/debug', binaryName)
}

async function buildPublicationContainerImage(tag) {
  await ensureDockerAvailable()
  await runChecked('docker', [
    'build',
    '-f',
    path.join(repoRoot, 'docker/publication/Dockerfile'),
    '-t',
    tag,
    repoRoot,
  ], { env: process.env })
}

function startPublicationContainer({
  image,
  name,
  packageDir,
  workspaceDir,
  port,
}) {
  return startProcess('docker', [
    'run',
    '--rm',
    '--name',
    name,
    '-p',
    `127.0.0.1:${port}:3000`,
    '-v',
    `${packageDir}:/publication:ro`,
    '-v',
    `${workspaceDir}:/workspace`,
    '-e',
    'ARROBA_PUBLICATION_PACKAGE=/publication',
    '-e',
    'HOST=0.0.0.0',
    '-e',
    'PORT=3000',
    image,
    'standalone',
  ], process.env, name)
}

async function removeDockerContainer(name) {
  await run('docker', ['rm', '-f', name], { env: process.env })
}

async function removeDockerImage(tag) {
  await run('docker', ['image', 'rm', '-f', tag], { env: process.env })
}

async function createContainerPortablePackage(sourceDir, targetDir) {
  await rm(targetDir, { recursive: true, force: true })
  await cp(sourceDir, targetDir, { recursive: true })
  const snapshotPath = path.join(targetDir, 'workflow.snapshot.json')
  const snapshot = JSON.parse(await readFile(snapshotPath, 'utf8'))
  if (snapshot.source_session) {
    snapshot.source_session.workspace_id = '/workspace'
    snapshot.source_session.worktree_id = '/workspace'
  }
  for (const agent of snapshot.agents ?? []) {
    if (agent.workspace_id != null) agent.workspace_id = '/workspace'
    if (agent.worktree_id != null) agent.worktree_id = '/workspace'
  }
  await writeFile(snapshotPath, `${JSON.stringify(snapshot, null, 2)}\n`)
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

function startServeWithProviderPrompt({
  cliBinary,
  packageDir,
  port,
  kernelUrl,
  env,
  provider,
  model,
  effort,
}) {
  const script = `
set timeout 45
set cli $env(ARROBA_EXPECT_CLI_BINARY)
set package_dir $env(ARROBA_EXPECT_PUBLICATION_PACKAGE)
set port $env(ARROBA_EXPECT_PUBLICATION_PORT)
set kernel_url $env(ARROBA_EXPECT_KERNEL_URL)
set provider $env(ARROBA_EXPECT_REPLACEMENT_PROVIDER)
set model $env(ARROBA_EXPECT_REPLACEMENT_MODEL)
set effort $env(ARROBA_EXPECT_REPLACEMENT_EFFORT)
trap { catch { exec kill [exp_pid] }; exit 143 } SIGTERM
spawn -noecho $cli serve $package_dir $port --kernel-url $kernel_url
expect {
  -re {Replacement provider:} { send -- "$provider\\r" }
  timeout { puts stderr "timed out waiting for provider replacement prompt"; exit 2 }
  eof { set wait_result [wait]; exit [lindex $wait_result 3] }
}
expect {
  -re {Replacement model .*:} { send -- "$model\\r" }
  timeout { puts stderr "timed out waiting for model replacement prompt"; exit 2 }
  eof { set wait_result [wait]; exit [lindex $wait_result 3] }
}
expect {
  -re {Replacement effort .*:} { send -- "$effort\\r" }
  timeout { puts stderr "timed out waiting for effort replacement prompt"; exit 2 }
  eof { set wait_result [wait]; exit [lindex $wait_result 3] }
}
expect {
  -re {workflow gateway listening} { puts "EXPECT_SERVE_READY"; exp_continue }
  timeout { exp_continue }
  eof { set wait_result [wait]; exit [lindex $wait_result 3] }
}
`
  return startProcess('/usr/bin/expect', ['-c', script], {
    ...env,
    ARROBA_EXPECT_CLI_BINARY: cliBinary,
    ARROBA_EXPECT_PUBLICATION_PACKAGE: packageDir,
    ARROBA_EXPECT_PUBLICATION_PORT: String(port),
    ARROBA_EXPECT_KERNEL_URL: kernelUrl,
    ARROBA_EXPECT_REPLACEMENT_PROVIDER: provider,
    ARROBA_EXPECT_REPLACEMENT_MODEL: model,
    ARROBA_EXPECT_REPLACEMENT_EFFORT: effort,
  }, 'arroba-serve-provider-override')
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

async function findChromeExecutable() {
  const candidates = [
    process.env.ARROBA_CHROME_PATH,
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    'google-chrome',
    'chromium',
    'chromium-browser',
  ].filter(Boolean)
  for (const candidate of candidates) {
    const result = await run(candidate, ['--version'])
    if (result.code === 0) return candidate
  }
  return null
}

async function runHumanHttpBrowserDrill({ url, root, timeoutMs = 30_000 }) {
  const chromePath = await findChromeExecutable()
  if (!chromePath) {
    logStep('browser_screenshot_skipped', { reason: 'chrome-not-found' })
    return
  }
  const debuggingPort = await freePort()
  const userDataDir = path.join(root, 'chrome-profile')
  const screenshotPath = path.join(root, 'human-http-final.png')
  await mkdir(userDataDir, { recursive: true })
  const chrome = startProcess(
    chromePath,
    [
      '--headless=new',
      '--disable-gpu',
      '--disable-background-networking',
      '--disable-extensions',
      '--no-first-run',
      '--no-default-browser-check',
      '--no-sandbox',
      '--remote-debugging-address=127.0.0.1',
      `--remote-debugging-port=${debuggingPort}`,
      `--user-data-dir=${userDataDir}`,
      'about:blank',
    ],
    process.env,
    'chrome-human-http-publication',
  )
  let cdp = null
  try {
    const target = await waitForChromeTarget(debuggingPort, 'about:blank', chrome)
    cdp = await connectChromeTarget(target.webSocketDebuggerUrl)
    await cdp.send('Page.enable')
    await cdp.send('Runtime.enable')
    await cdp.send('Page.addScriptToEvaluateOnNewDocument', { source: browserStatusRecorderScript() })
    await cdp.send('Page.navigate', { url })
    const finalState = await waitForBrowserFinalOutput(cdp, timeoutMs)
    const statuses = finalState.statuses ?? []
    const outputs = finalState.outputs ?? []
    for (const expectedStatus of ['Running', 'Completed']) {
      if (!statuses.includes(expectedStatus)) {
        throw new Error(`browser did not observe ${expectedStatus} status; statuses=${JSON.stringify(statuses)}`)
      }
    }
    for (const expectedValue of ['"value":1841', '"value":1842']) {
      if (!outputs.some((output) => output.includes(expectedValue))) {
        throw new Error(`browser did not observe ${expectedValue} output; outputs=${JSON.stringify(outputs)}`)
      }
    }
    const screenshot = await cdp.send('Page.captureScreenshot', { format: 'png', fromSurface: true })
    if (typeof screenshot.data !== 'string' || screenshot.data.length < 1000) {
      throw new Error('browser screenshot was empty')
    }
    await writeFile(screenshotPath, Buffer.from(screenshot.data, 'base64'))
    logStep('browser_screenshot_ok', { screenshotPath, statuses, outputs })
  } finally {
    await cdp?.close?.().catch(() => {})
    await stopProcess(chrome)
  }
}

async function runHumanHttpRootFormBrowserDrill({ baseUrl, root, timeoutMs = 30_000 }) {
  const chromePath = await findChromeExecutable()
  if (!chromePath) {
    logStep('browser_root_form_screenshot_skipped', { reason: 'chrome-not-found' })
    return
  }
  const debuggingPort = await freePort()
  const userDataDir = path.join(root, 'chrome-root-form-profile')
  const screenshotPath = path.join(root, 'human-http-root-form-final.png')
  const artifactPath = path.join(root, 'human-http-root-form-upload.txt')
  await mkdir(userDataDir, { recursive: true })
  await writeFile(artifactPath, 'root-form-publication-upload\n')
  const chrome = startProcess(
    chromePath,
    [
      '--headless=new',
      '--disable-gpu',
      '--disable-background-networking',
      '--disable-extensions',
      '--no-first-run',
      '--no-default-browser-check',
      '--no-sandbox',
      '--remote-debugging-address=127.0.0.1',
      `--remote-debugging-port=${debuggingPort}`,
      `--user-data-dir=${userDataDir}`,
      'about:blank',
    ],
    process.env,
    'chrome-human-http-root-form-publication',
  )
  let cdp = null
  try {
    const target = await waitForChromeTarget(debuggingPort, 'about:blank', chrome)
    cdp = await connectChromeTarget(target.webSocketDebuggerUrl)
    await cdp.send('Page.enable')
    await cdp.send('Runtime.enable')
    await cdp.send('DOM.enable')
    await cdp.send('Page.addScriptToEvaluateOnNewDocument', { source: browserStatusRecorderScript() })
    await cdp.send('Page.navigate', { url: baseUrl })
    await waitForBrowserRootForm(cdp)
    const document = await cdp.send('DOM.getDocument')
    const input = await cdp.send('DOM.querySelector', {
      nodeId: document.root.nodeId,
      selector: 'input[type="file"][name="artifact"]',
    })
    if (!input.nodeId) {
      throw new Error('browser root form did not expose artifact file input')
    }
    await cdp.send('DOM.setFileInputFiles', {
      nodeId: input.nodeId,
      files: [artifactPath],
    })
    const submitted = await cdp.send('Runtime.evaluate', {
      returnByValue: true,
      expression: `(() => {
        const form = document.querySelector('#invoke-form');
        const prompt = form?.querySelector('[name="prompt"]');
        if (!form || !prompt) return false;
        prompt.value = 'browser-root-form-publication';
        form.requestSubmit();
        return true;
      })()`,
    })
    if (submitted.result?.value !== true) {
      throw new Error('browser root form could not be submitted')
    }
    const finalState = await waitForBrowserFinalOutput(cdp, timeoutMs)
    const statuses = finalState.statuses ?? []
    const outputs = finalState.outputs ?? []
    if (finalState.status !== 'Completed' && !statuses.includes('Completed')) {
      throw new Error(`browser root form did not complete; state=${JSON.stringify(finalState)}`)
    }
    for (const expectedValue of ['"value":1841', '"value":1842']) {
      if (!outputs.some((output) => output.includes(expectedValue)) && !String(finalState.output ?? '').includes(expectedValue)) {
        throw new Error(`browser root form did not observe ${expectedValue} output; outputs=${JSON.stringify(outputs)}, state=${JSON.stringify(finalState)}`)
      }
    }
    const screenshot = await cdp.send('Page.captureScreenshot', { format: 'png', fromSurface: true })
    if (typeof screenshot.data !== 'string' || screenshot.data.length < 1000) {
      throw new Error('browser root form screenshot was empty')
    }
    await writeFile(screenshotPath, Buffer.from(screenshot.data, 'base64'))
    logStep('browser_root_form_screenshot_ok', { screenshotPath, statuses, outputs })
  } finally {
    await cdp?.close?.().catch(() => {})
    await stopProcess(chrome)
  }
}

async function waitForBrowserRootForm(cdp) {
  const deadline = Date.now() + 20_000
  let lastState = null
  while (Date.now() < deadline) {
    const evaluated = await cdp.send('Runtime.evaluate', {
      returnByValue: true,
      expression: `(() => {
        const form = document.querySelector('#invoke-form');
        const prompt = form?.querySelector('[name="prompt"]');
        const file = form?.querySelector('input[type="file"][name="artifact"]');
        return {
          title: document.title,
          hasForm: !!form,
          hasPrompt: !!prompt,
          hasFile: !!file,
        };
      })()`,
    })
    lastState = evaluated.result?.value ?? null
    if (lastState?.hasForm && lastState?.hasPrompt && lastState?.hasFile) return
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`browser root form did not render: ${JSON.stringify(lastState)}`)
}

async function waitForChromeTarget(debuggingPort, expectedUrl, chrome) {
  const endpoint = `http://127.0.0.1:${debuggingPort}/json/list`
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const response = await fetch(endpoint)
      const targets = await response.json()
      const target = targets.find((candidate) => candidate.type === 'page' && candidate.url === expectedUrl)
        ?? targets.find((candidate) => candidate.type === 'page' && candidate.webSocketDebuggerUrl)
      if (target?.webSocketDebuggerUrl) return target
    } catch (error) {
      lastError = error
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`Chrome DevTools target did not become ready: ${lastError?.message ?? 'no page target'}\n${chrome.logs.stderr.slice(-2_000)}`)
}

async function connectChromeTarget(webSocketUrl) {
  const socket = new WebSocket(webSocketUrl)
  let nextId = 1
  const pending = new Map()
  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out opening Chrome DevTools socket')), 10_000)
    socket.once('open', () => {
      clearTimeout(timeout)
      resolve()
    })
    socket.once('error', reject)
  })
  socket.on('message', (data) => {
    const message = JSON.parse(data.toString())
    if (typeof message.id !== 'number') return
    const waiter = pending.get(message.id)
    if (!waiter) return
    pending.delete(message.id)
    if (message.error) waiter.reject(new Error(`${message.error.message}: ${message.error.data ?? ''}`))
    else waiter.resolve(message.result ?? {})
  })
  socket.on('error', (error) => {
    for (const waiter of pending.values()) waiter.reject(error)
    pending.clear()
  })
  return {
    send(method, params = {}) {
      const id = nextId++
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        socket.send(JSON.stringify({ id, method, params }))
      })
    },
    close() {
      return new Promise((resolve) => {
        if (socket.readyState === WebSocket.CLOSED) return resolve()
        socket.once('close', resolve)
        socket.close()
      })
    },
  }
}

async function waitForBrowserFinalOutput(cdp, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  let lastState = null
  const polledStatuses = []
  const polledOutputs = []
  while (Date.now() < deadline) {
    const evaluated = await cdp.send('Runtime.evaluate', {
      returnByValue: true,
      expression: `(() => {
        const status = document.querySelector('#status')?.textContent?.trim() || '';
        const output = document.querySelector('#output')?.textContent?.trim() || '';
        const statuses = Array.isArray(window.__arrobaPublicationDrillStatuses) ? window.__arrobaPublicationDrillStatuses : [];
        const outputs = Array.isArray(window.__arrobaPublicationDrillOutputs) ? window.__arrobaPublicationDrillOutputs : [];
        return { status, output, statuses, outputs, title: document.title, ok: status === 'Completed' && output.includes('"value":1842') };
      })()`,
    })
    lastState = evaluated.result?.value ?? null
    if (lastState?.status && !polledStatuses.includes(lastState.status)) {
      polledStatuses.push(lastState.status)
    }
    if (lastState?.output && !polledOutputs.includes(lastState.output)) {
      polledOutputs.push(lastState.output)
    }
    if (lastState?.ok) {
      return {
        ...lastState,
        statuses: [...new Set([...polledStatuses, ...(lastState.statuses ?? [])])],
        outputs: [...new Set([...polledOutputs, ...(lastState.outputs ?? [])])],
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  throw new Error(`browser did not render final workflow output: ${JSON.stringify(lastState)}`)
}

function browserStatusRecorderScript() {
  return `
    (() => {
      const statuses = [];
      const outputs = [];
      let last = null;
      let lastOutput = null;
      const NativeEventSource = window.EventSource;
      Object.defineProperty(window, '__arrobaPublicationDrillStatuses', {
        value: statuses,
        configurable: true,
      });
      Object.defineProperty(window, '__arrobaPublicationDrillOutputs', {
        value: outputs,
        configurable: true,
      });
      if (typeof NativeEventSource === 'function') {
        window.EventSource = function(...args) {
          const source = new NativeEventSource(...args);
          source.addEventListener('partial', (event) => {
            try {
              const message = JSON.parse(event.data).message;
              if (typeof message === 'string' && message) outputs.push(message);
            } catch {
            }
          });
          source.addEventListener('final', (event) => {
            try {
              const message = JSON.parse(event.data).workflow_run?.final_output?.message;
              if (typeof message === 'string' && message) outputs.push(message);
            } catch {
            }
          });
          return source;
        };
        window.EventSource.prototype = NativeEventSource.prototype;
      }
      const record = () => {
        const status = document.querySelector('#status')?.textContent?.trim();
        if (status && status !== last) {
          last = status;
          statuses.push(status);
        }
        const output = document.querySelector('#output')?.textContent?.trim();
        if (output && output !== lastOutput) {
          lastOutput = output;
          outputs.push(output);
        }
      };
      const install = () => {
        record();
        const statusEl = document.querySelector('#status');
        if (statusEl) {
          new MutationObserver(record).observe(statusEl, { childList: true, subtree: true, characterData: true });
        }
      };
      if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', install, { once: true });
      } else {
        install();
      }
    })();
  `
}

async function waitForProcessExit(child, timeoutMs = 10_000) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return { code: child.exitCode, signal: child.signalCode }
  }
  return await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(new Error(`${child.name ?? 'process'} did not exit within ${timeoutMs}ms`))
    }, timeoutMs)
    child.once('exit', (code, signal) => {
      clearTimeout(timeout)
      resolve({ code, signal })
    })
  })
}

async function assertGatewayDoesNotListen(baseUrl, timeoutMs = 1_500) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${baseUrl}/health`)
      if (response.ok) {
        throw new Error(`gateway unexpectedly listened at ${baseUrl}`)
      }
    } catch (error) {
      if (/unexpectedly listened/.test(error.message)) throw error
    }
    await new Promise((resolve) => setTimeout(resolve, 150))
  }
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

async function waitForGateway(baseUrl, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${baseUrl}/health`)
      if (response.ok) return
      lastError = new Error(`health status ${response.status}`)
    } catch (error) {
      lastError = error
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`gateway did not become ready: ${lastError?.message ?? String(lastError)}`)
}

async function waitForContainerGateway(baseUrl, containerProcess, timeoutMs = 60_000) {
  try {
    await waitForGateway(baseUrl, timeoutMs)
  } catch (error) {
    throw new Error([
      `container gateway did not become ready: ${error instanceof Error ? error.message : String(error)}`,
      `stdout:\n${containerProcess?.logs?.stdout ?? ''}`,
      `stderr:\n${containerProcess?.logs?.stderr ?? ''}`,
    ].join('\n'))
  }
}

async function assertPublicationRuntimeSessionHidden(client, runtimeSessionId) {
  const session = variant(await client.send(getSessionStateRequest(runtimeSessionId)), 'SessionState').session
  if (session?.id !== runtimeSessionId || session.hidden !== true) {
    throw new Error(`expected publication runtime session ${runtimeSessionId} to be hidden, got ${JSON.stringify(session)}`)
  }
  const sessions = variant(await client.send(listSessionsRequest()), 'SessionsListed').sessions ?? []
  if (sessions.some((candidate) => candidate.id === runtimeSessionId)) {
    throw new Error(`publication runtime session ${runtimeSessionId} leaked into normal session list`)
  }
}

async function assertPackageDoesNotContain(exportDir, forbiddenValues) {
  const forbidden = forbiddenValues.filter((value) => typeof value === 'string' && value.length > 0)
  async function visit(dir) {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const entryPath = path.join(dir, entry.name)
      if (entry.isDirectory()) {
        await visit(entryPath)
        continue
      }
      if (!entry.isFile()) continue
      const contents = await readFile(entryPath, 'utf8').catch(() => null)
      if (contents == null) continue
      for (const value of forbidden) {
        if (contents.includes(value)) {
          throw new Error(`publication package file ${entryPath} contains forbidden runtime token material`)
        }
      }
    }
  }
  await visit(exportDir)
}

async function createUnavailableProviderPackage(sourceDir, targetDir) {
  await rm(targetDir, { recursive: true, force: true })
  await cp(sourceDir, targetDir, { recursive: true })
  await rm(path.join(targetDir, 'bindings.local.json'), { force: true })
  const snapshotPath = path.join(targetDir, 'workflow.snapshot.json')
  const snapshot = JSON.parse(await readFile(snapshotPath, 'utf8'))
  const agent = snapshot.agents?.[0]
  if (!agent?.id) {
    throw new Error(`exported publication snapshot has no agent to mutate: ${JSON.stringify(snapshot.agents)}`)
  }
  agent.provider = 'missing-publication-provider'
  agent.model = 'missing-publication-model'
  agent.effort = 'missing-publication-effort'
  await writeFile(snapshotPath, `${JSON.stringify(snapshot, null, 2)}\n`)
  return { agentId: agent.id }
}

async function waitForTcpPort(host, port, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      await new Promise((resolve, reject) => {
        const socket = net.createConnection({ host, port }, () => {
          socket.end()
          resolve()
        })
        socket.once('error', reject)
      })
      return
    } catch (error) {
      lastError = error
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }
  throw new Error(`tcp port ${host}:${port} did not become ready: ${lastError?.message ?? String(lastError)}`)
}

async function waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    const relayClient = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await Promise.race([
        relayClient.send(listSessionsRequest()),
        new Promise((_, reject) => setTimeout(() => reject(new Error('relay probe timeout')), 2_000)),
      ])
      await relayClient.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await relayClient.close().catch(() => {})
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }
  throw new Error(`relay target ${targetDaemonAlias} did not become reachable: ${lastError?.message ?? String(lastError)}`)
}

async function waitForRegisteredPublicationEndpoint(client, sessionId, publicationId, expectedLocalUrl, expectedOpenUrlPrefix) {
  const deadline = Date.now() + 20_000
  let lastPublication = null
  while (Date.now() < deadline) {
    const response = variant(
      await client.send(getWorkflowPublicationRequest(sessionId, publicationId)),
      'WorkflowPublication',
    )
    lastPublication = response.publication ?? null
    if (
      lastPublication?.status === 'running'
      && lastPublication.deployment?.local_url === expectedLocalUrl
      && typeof lastPublication.open_url === 'string'
      && lastPublication.open_url.startsWith(expectedOpenUrlPrefix)
    ) {
      return lastPublication
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`publication endpoint did not register as running at ${expectedLocalUrl}: ${JSON.stringify(lastPublication)}`)
}

async function waitForProviderRunReady(client, providerRunId) {
  const deadline = Date.now() + 20_000
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

async function createDeterministicPublicationSession(client, sessionIds, options) {
  const session = variant(
    await client.send(createSessionRequest(options.workspace, options.workspace, options.sessionAlias)),
    'SessionCreated',
  ).session
  sessionIds.push(session.id)
  await client.send(attachToSessionRequest(session.id, `${options.attachAlias}-${process.pid}`))
  const agent = variant(
    await client.send(spawnAgentRequest(session.id, 'dev-stub', options.agentAlias, 'workflow-intermediate-node', options.workspace, 'low')),
    'AgentSpawned',
  ).agent
  const providerRun = variant(
    await client.send(launchProviderRunRequest(session.id, 'dev-stub', 'default', 'workflow-intermediate-node', 'low', agent.id)),
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(client, providerRun.id)
  const workflow = variant(await client.send(createWorkflowRequest(session.id, options.workflowAlias)), 'WorkflowCreated').workflow
  const node = variant(await client.send(addWorkflowNodeRequest(session.id, workflow.id, agent.id)), 'WorkflowNodeAdded').node
  await client.send(updateWorkflowNodeInstructionsRequest(
    session.id,
    workflow.id,
    node.id,
    'Complete this publication drill workflow with the deterministic final output envelope emitted by the dev stub.',
  ))
  await client.send(setWorkflowNodeCanCompleteRunRequest(session.id, workflow.id, node.id, true))
  await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(session.id, workflow.id, node.id, true))
  const endpoint = variant(
    await client.send(createWorkflowEndpointRequest(session.id, workflow.id, node.id, options.endpointAlias)),
    'WorkflowEndpointCreated',
  ).endpoint
  const publication = variant(
    await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
      alias: options.publicationAlias,
      route: options.route,
      methods: options.methods,
      transport: { kind: options.transportKind },
      parser: { kind: 'json' },
      mode: 'async',
    })),
    'WorkflowPublicationCreated',
  ).publication
  return { session, publication }
}

async function waitForWatchdogWorkflowRun(client, sessionId, workflowId, options = {}) {
  const deadline = Date.now() + (options.requireOutput ? 30_000 : 20_000)
  let lastRuns = []
  let lastRun = null
  while (Date.now() < deadline) {
    const listed = variant(
      await client.send(listWorkflowRunsRequest(sessionId, workflowId)),
      'WorkflowRunsListed',
    )
    lastRuns = listed.workflow_runs ?? []
    if (lastRuns.length > 0) {
      const candidate = lastRuns[0]
      const detailed = variant(
        await client.send(getWorkflowRunRequest(sessionId, candidate.id)),
        'WorkflowRun',
      ).workflow_run ?? candidate
      lastRun = detailed
      if (!options.requireOutput || (isTerminalWorkflowRunStatus(detailed.status) && detailed.final_output?.message)) {
        return detailed
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  let activeClaims = []
  try {
    const health = variant(await client.send(getDaemonHealthRequest()), 'DaemonHealth')
    activeClaims = health.projection?.workspace_coordination?.active_operation_claims ?? []
  } catch {
    activeClaims = []
  }
  throw new Error(`watchdog publication did not reach expected run state; active claims: ${JSON.stringify(activeClaims)}, last run: ${JSON.stringify(lastRun)}, last runs: ${JSON.stringify(lastRuns)}`)
}

async function waitForPublicationStatusLatestOutput(gatewayUrl, expectedMessage) {
  const deadline = Date.now() + 30_000
  let lastStatus = null
  while (Date.now() < deadline) {
    const response = await fetch(`${gatewayUrl}/.well-known/arroba/publication/status`)
    lastStatus = await response.json()
    if (response.status === 200 && lastStatus.latest_output?.kind === 'final' && lastStatus.latest_output?.message === expectedMessage) {
      return lastStatus
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`expected publication status latest final output ${expectedMessage}, got ${JSON.stringify(lastStatus)}`)
}

async function runLivePublicationManifestMode({
  manifestPath,
  client,
  env,
  kernelUrl,
  relayPort,
  publications,
}) {
  const gateways = []
  try {
    const manifest = {
      generated_at_ms: Date.now(),
      relay_display_prefix: `http://127.0.0.1:${relayPort}/display/`,
      publications: {},
    }
    for (const item of publications) {
      const port = await freePort()
      const localUrl = `http://127.0.0.1:${port}`
      const gateway = startProcess(
        process.execPath,
        [path.join(repoRoot, 'apps/server/dist/index.js')],
        {
          ...env,
          HOST: '127.0.0.1',
          PORT: String(port),
          ARROBA_KERNEL_URL: kernelUrl,
          ARROBA_PUBLICATION_SESSION_ID: item.sessionId,
          ARROBA_PUBLICATION_ID: item.publication.id,
        },
        `gateway-live-${item.key}`,
      )
      gateways.push(gateway)
      await waitForGateway(localUrl)
      const registered = await waitForRegisteredPublicationEndpoint(
        client,
        item.sessionId,
        item.publication.id,
        `${localUrl}/`,
        `http://127.0.0.1:${relayPort}/display/publication-`,
      )
      manifest.publications[item.key] = {
        id: item.publication.id,
        alias: item.publication.alias ?? null,
        route: item.publication.route ?? null,
        transport: item.transport,
        local_url: localUrl,
        open_url: registered.open_url,
        session_id: item.sessionId,
      }
    }
    await mkdir(path.dirname(manifestPath), { recursive: true })
    await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8')
    logStep('live_publication_manifest_ready', { manifestPath, publications: Object.keys(manifest.publications) })
    await waitForStopSignal()
  } finally {
    await Promise.all(gateways.map((gateway) => stopProcess(gateway).catch(() => {})))
  }
}

async function waitForStopSignal() {
  await new Promise((resolve) => {
    const done = () => {
      process.off('SIGTERM', done)
      process.off('SIGINT', done)
      resolve()
    }
    process.once('SIGTERM', done)
    process.once('SIGINT', done)
  })
}

function isTerminalWorkflowRunStatus(status) {
  return ['completed', 'failed', 'stopped'].includes(String(status).toLowerCase())
}

async function createSelfSignedCertificate(root) {
  const keyFile = path.join(root, 'gateway.key')
  const certFile = path.join(root, 'gateway.crt')
  const args = [
    'req',
    '-x509',
    '-newkey',
    'rsa:2048',
    '-nodes',
    '-keyout',
    keyFile,
    '-out',
    certFile,
    '-subj',
    '/CN=127.0.0.1',
    '-addext',
    'subjectAltName=IP:127.0.0.1,DNS:localhost',
    '-days',
    '1',
  ]
  let result = await run('openssl', args, { cwd: root })
  if (result.code !== 0 && result.stderr.includes('addext')) {
    result = await run('openssl', args.filter((arg, index) => arg !== '-addext' && args[index - 1] !== '-addext'), { cwd: root })
  }
  if (result.code !== 0) {
    throw new Error(`openssl self-signed certificate generation failed\n${result.stdout}\n${result.stderr}`)
  }
  return { keyFile, certFile }
}

async function main() {
  const drillTmpRoot = process.env.ARROBA_PUBLICATION_DRILL_TMPDIR
    ?? path.join(repoRoot, '.tmp')
  await mkdir(drillTmpRoot, { recursive: true })
  const root = await mkdtemp(path.join(drillTmpRoot, 'arroba-publication-drill-'))
  const workspace = path.join(root, 'workspace')
  const apiSseWorkspace = path.join(root, 'api-sse-workspace')
  const apiSseTunnelWorkspace = path.join(root, 'api-sse-tunnel-workspace')
  const websocketWorkspace = path.join(root, 'websocket-workspace')
  const websocketTunnelWorkspace = path.join(root, 'websocket-tunnel-workspace')
  const browserWorkspace = path.join(root, 'browser-workspace')
  const browserRootWorkspace = path.join(root, 'browser-root-workspace')
  const mcpWorkspace = path.join(root, 'mcp-workspace')
  const watchdogWorkspace = path.join(root, 'watchdog-workspace')
  const home = path.join(root, 'home')
  const configHome = path.join(root, 'config')
  const stateHome = path.join(root, 'state')
  const relayPort = await freePort()
  const kernelPort = await freePort()
  const mcpPort = await freePort()
  const opencodePort = await freePort()
  const codexPort = await freePort()
  const gatewayPort = await freePort()
  const gatewayHttpsPort = await freePort()
  const relayUrl = `ws://127.0.0.1:${relayPort}`
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const gatewayUrl = `http://127.0.0.1:${gatewayPort}`
  const gatewayHttpsUrl = `https://127.0.0.1:${gatewayHttpsPort}`
  const relayToken = `publication-drill-relay-${process.pid}`
  const daemonAlias = `publication-drill-${process.pid}`
  const env = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: stateHome,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(opencodePort),
    ARROBA_CODEX_PORT: String(codexPort),
    ARROBA_DAEMON_ID: daemonAlias,
    ARROBA_DAEMON_ALIAS: daemonAlias,
    ARROBA_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(root, 'history'),
    ARROBA_RELAY_URL: relayUrl,
    ARROBA_RELAY_TOKEN: relayToken,
  }

  let relay = null
  let kernel = null
  let gateway = null
  let client = null
  const sessionIds = []
  const dockerContainers = []
  const dockerImages = []
  let succeeded = false
  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(apiSseWorkspace, { recursive: true })
    await mkdir(apiSseTunnelWorkspace, { recursive: true })
    await mkdir(websocketWorkspace, { recursive: true })
    await mkdir(websocketTunnelWorkspace, { recursive: true })
    await mkdir(browserWorkspace, { recursive: true })
    await mkdir(browserRootWorkspace, { recursive: true })
    await mkdir(mcpWorkspace, { recursive: true })
    await mkdir(watchdogWorkspace, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })
    await writeFile(path.join(configHome, 'arroba', 'config.toml'), 'version = 1\n', 'utf8')
    const tls = await createSelfSignedCertificate(root)

    const kernelBinary = await buildRustBinary('arroba-kernel')
    const cliBinary = await buildRustBinary('arroba-cli')
    const relayBinary = await buildRustBinary('arroba-relay')
    relay = startProcess(relayBinary, [], {
      ...env,
      ARROBA_RELAY_HOST: '127.0.0.1',
      ARROBA_RELAY_PORT: String(relayPort),
      ARROBA_RELAY_TOKEN: relayToken,
    }, 'relay')
    await waitForTcpPort('127.0.0.1', relayPort)
    kernel = startProcess(kernelBinary, [], env, 'kernel')
    await waitForKernel(kernelUrl)
    await waitForRelayTarget(relayUrl, relayToken, daemonAlias)
    client = new LocalIpcClient(kernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })

    logStep('create_session')
    const session = variant(await client.send(createSessionRequest(workspace, workspace, 'publication-drill')), 'SessionCreated').session
    sessionIds.push(session.id)
    await client.send(attachToSessionRequest(session.id, `publication-drill-${process.pid}`))

    logStep('spawn_agent')
    const agent = variant(
      await client.send(spawnAgentRequest(session.id, 'dev-stub', 'publisher', 'publication-drill-model', workspace, 'low')),
      'AgentSpawned',
    ).agent
    const providerRun = variant(
      await client.send(launchProviderRunRequest(session.id, 'dev-stub', 'default', 'publication-drill-model', 'low', agent.id)),
      'ProviderRunLaunchAccepted',
    ).provider_run
    await waitForProviderRunReady(client, providerRun.id)

    logStep('create_workflow')
    const workflow = variant(await client.send(createWorkflowRequest(session.id, 'published')), 'WorkflowCreated').workflow
    const node = variant(await client.send(addWorkflowNodeRequest(session.id, workflow.id, agent.id)), 'WorkflowNodeAdded').node
    await client.send(updateWorkflowNodeInstructionsRequest(
      session.id,
      workflow.id,
      node.id,
      'For the publication drill, acknowledge the request and complete the workflow.',
    ))
    const endpoint = variant(
      await client.send(createWorkflowEndpointRequest(session.id, workflow.id, node.id, 'http')),
      'WorkflowEndpointCreated',
    ).endpoint

    logStep('create_publication')
    const publication = variant(
      await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
        alias: 'public_http',
        route: '/qa/*',
        methods: ['GET'],
        parser: { kind: 'path_template', template: '/qa/:task' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication

    logStep('create_api_sse_publication')
    const apiSsePublication = variant(
      await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
        alias: 'public_api_sse',
        route: '/invoke',
        methods: ['POST'],
        transport: { kind: 'api_sse_json' },
        parser: { kind: 'json' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication

    logStep('create_api_sse_final_session')
    const apiSseSession = variant(
      await client.send(createSessionRequest(apiSseWorkspace, apiSseWorkspace, 'publication-drill-api-sse-final')),
      'SessionCreated',
    ).session
    sessionIds.push(apiSseSession.id)
    await client.send(attachToSessionRequest(apiSseSession.id, `publication-drill-api-sse-final-${process.pid}`))
    const apiSseAgent = variant(
      await client.send(spawnAgentRequest(apiSseSession.id, 'dev-stub', 'api-sse-final', 'workflow-intermediate-node', apiSseWorkspace, 'low')),
      'AgentSpawned',
    ).agent
    const apiSseProviderRun = variant(
      await client.send(launchProviderRunRequest(apiSseSession.id, 'dev-stub', 'default', 'workflow-intermediate-node', 'low', apiSseAgent.id)),
      'ProviderRunLaunchAccepted',
    ).provider_run
    await waitForProviderRunReady(client, apiSseProviderRun.id)
    const apiSseWorkflow = variant(await client.send(createWorkflowRequest(apiSseSession.id, 'published-api-sse-final')), 'WorkflowCreated').workflow
    const apiSseNode = variant(await client.send(addWorkflowNodeRequest(apiSseSession.id, apiSseWorkflow.id, apiSseAgent.id)), 'WorkflowNodeAdded').node
    await client.send(updateWorkflowNodeInstructionsRequest(
      apiSseSession.id,
      apiSseWorkflow.id,
      apiSseNode.id,
      'Complete this publication drill workflow with the deterministic final output envelope emitted by the dev stub.',
    ))
    await client.send(setWorkflowNodeCanCompleteRunRequest(apiSseSession.id, apiSseWorkflow.id, apiSseNode.id, true))
    await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(apiSseSession.id, apiSseWorkflow.id, apiSseNode.id, true))
    const apiSseFinalEndpoint = variant(
      await client.send(createWorkflowEndpointRequest(apiSseSession.id, apiSseWorkflow.id, apiSseNode.id, 'api')),
      'WorkflowEndpointCreated',
    ).endpoint
    const apiSseFinalPublication = variant(
      await client.send(createWorkflowPublicationRequest(apiSseSession.id, apiSseWorkflow.id, apiSseFinalEndpoint.id, {
        alias: 'public_api_sse_final',
        route: '/invoke',
        methods: ['POST'],
        transport: { kind: 'api_sse_json' },
        parser: { kind: 'json' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication
    logStep('create_api_sse_tunnel_session')
    const apiSseTunnel = await createDeterministicPublicationSession(client, sessionIds, {
      workspace: apiSseTunnelWorkspace,
      sessionAlias: 'publication-drill-api-sse-tunnel-final',
      attachAlias: 'publication-drill-api-sse-tunnel-final',
      agentAlias: 'api-sse-tunnel-final',
      workflowAlias: 'published-api-sse-tunnel-final',
      endpointAlias: 'api-tunnel',
      publicationAlias: 'public_api_sse_tunnel_final',
      route: '/invoke',
      methods: ['POST'],
      transportKind: 'api_sse_json',
    })
    logStep('create_websocket_session')
    const websocketSession = variant(
      await client.send(createSessionRequest(websocketWorkspace, websocketWorkspace, 'publication-drill-websocket-final')),
      'SessionCreated',
    ).session
    sessionIds.push(websocketSession.id)
    await client.send(attachToSessionRequest(websocketSession.id, `publication-drill-websocket-final-${process.pid}`))
    const websocketAgent = variant(
      await client.send(spawnAgentRequest(websocketSession.id, 'dev-stub', 'websocket-final', 'workflow-intermediate-node', websocketWorkspace, 'low')),
      'AgentSpawned',
    ).agent
    const websocketProviderRun = variant(
      await client.send(launchProviderRunRequest(websocketSession.id, 'dev-stub', 'default', 'workflow-intermediate-node', 'low', websocketAgent.id)),
      'ProviderRunLaunchAccepted',
    ).provider_run
    await waitForProviderRunReady(client, websocketProviderRun.id)
    const websocketWorkflow = variant(await client.send(createWorkflowRequest(websocketSession.id, 'published-websocket-final')), 'WorkflowCreated').workflow
    const websocketNode = variant(await client.send(addWorkflowNodeRequest(websocketSession.id, websocketWorkflow.id, websocketAgent.id)), 'WorkflowNodeAdded').node
    await client.send(updateWorkflowNodeInstructionsRequest(
      websocketSession.id,
      websocketWorkflow.id,
      websocketNode.id,
      'Complete this publication drill workflow with the deterministic final output envelope emitted by the dev stub.',
    ))
    await client.send(setWorkflowNodeCanCompleteRunRequest(websocketSession.id, websocketWorkflow.id, websocketNode.id, true))
    await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(websocketSession.id, websocketWorkflow.id, websocketNode.id, true))
    const websocketEndpoint = variant(
      await client.send(createWorkflowEndpointRequest(websocketSession.id, websocketWorkflow.id, websocketNode.id, 'websocket')),
      'WorkflowEndpointCreated',
    ).endpoint
    const websocketFinalPublication = variant(
      await client.send(createWorkflowPublicationRequest(websocketSession.id, websocketWorkflow.id, websocketEndpoint.id, {
        alias: 'public_websocket_final',
        route: '/.well-known/arroba/publication/ws',
        methods: ['GET'],
        transport: { kind: 'websocket_json' },
        parser: { kind: 'json' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication
    logStep('create_websocket_tunnel_session')
    const websocketTunnel = await createDeterministicPublicationSession(client, sessionIds, {
      workspace: websocketTunnelWorkspace,
      sessionAlias: 'publication-drill-websocket-tunnel-final',
      attachAlias: 'publication-drill-websocket-tunnel-final',
      agentAlias: 'websocket-tunnel-final',
      workflowAlias: 'published-websocket-tunnel-final',
      endpointAlias: 'websocket-tunnel',
      publicationAlias: 'public_websocket_tunnel_final',
      route: '/.well-known/arroba/publication/ws',
      methods: ['GET'],
      transportKind: 'websocket_json',
    })
    logStep('create_browser_session')
    const browserSession = variant(
      await client.send(createSessionRequest(browserWorkspace, browserWorkspace, 'publication-drill-human-http-final')),
      'SessionCreated',
    ).session
    sessionIds.push(browserSession.id)
    await client.send(attachToSessionRequest(browserSession.id, `publication-drill-human-http-final-${process.pid}`))
    const browserAgent = variant(
      await client.send(spawnAgentRequest(browserSession.id, 'dev-stub', 'human-http-final', 'workflow-intermediate-node', browserWorkspace, 'low')),
      'AgentSpawned',
    ).agent
    const browserProviderRun = variant(
      await client.send(launchProviderRunRequest(browserSession.id, 'dev-stub', 'default', 'workflow-intermediate-node', 'low', browserAgent.id)),
      'ProviderRunLaunchAccepted',
    ).provider_run
    await waitForProviderRunReady(client, browserProviderRun.id)
    const browserWorkflow = variant(await client.send(createWorkflowRequest(browserSession.id, 'published-human-http-final')), 'WorkflowCreated').workflow
    const browserNode = variant(await client.send(addWorkflowNodeRequest(browserSession.id, browserWorkflow.id, browserAgent.id)), 'WorkflowNodeAdded').node
    await client.send(updateWorkflowNodeInstructionsRequest(
      browserSession.id,
      browserWorkflow.id,
      browserNode.id,
      'Complete this publication drill workflow with the deterministic final output envelope emitted by the dev stub.',
    ))
    await client.send(setWorkflowNodeCanCompleteRunRequest(browserSession.id, browserWorkflow.id, browserNode.id, true))
    await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(browserSession.id, browserWorkflow.id, browserNode.id, true))
    const browserEndpoint = variant(
      await client.send(createWorkflowEndpointRequest(browserSession.id, browserWorkflow.id, browserNode.id, 'browser')),
      'WorkflowEndpointCreated',
    ).endpoint
    const humanHttpFinalPublication = variant(
      await client.send(createWorkflowPublicationRequest(browserSession.id, browserWorkflow.id, browserEndpoint.id, {
        alias: 'public_human_http_final',
        route: '/final/*',
        methods: ['GET'],
        transport: { kind: 'human_http' },
        parser: { kind: 'path_template', template: '/final/:task' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication
    logStep('create_browser_root_form_session')
    const browserRoot = await createDeterministicPublicationSession(client, sessionIds, {
      workspace: browserRootWorkspace,
      sessionAlias: 'publication-drill-human-http-root-form',
      attachAlias: 'publication-drill-human-http-root-form',
      agentAlias: 'human-http-root-form',
      workflowAlias: 'published-human-http-root-form',
      endpointAlias: 'browser-root',
      publicationAlias: 'public_human_http_root_form',
      route: '/final/*',
      methods: ['GET'],
      transportKind: 'human_http',
    })
    logStep('create_mcp_session')
    const mcpSession = variant(
      await client.send(createSessionRequest(mcpWorkspace, mcpWorkspace, 'publication-drill-mcp')),
      'SessionCreated',
    ).session
    sessionIds.push(mcpSession.id)
    await client.send(attachToSessionRequest(mcpSession.id, `publication-drill-mcp-${process.pid}`))
    const mcpAgent = variant(
      await client.send(spawnAgentRequest(mcpSession.id, 'dev-stub', 'mcp-final', 'workflow-single-turn-node', mcpWorkspace, 'low')),
      'AgentSpawned',
    ).agent
    const mcpProviderRun = variant(
      await client.send(launchProviderRunRequest(mcpSession.id, 'dev-stub', 'default', 'workflow-single-turn-node', 'low', mcpAgent.id)),
      'ProviderRunLaunchAccepted',
    ).provider_run
    await waitForProviderRunReady(client, mcpProviderRun.id)
    const mcpWorkflow = variant(await client.send(createWorkflowRequest(mcpSession.id, 'published-mcp')), 'WorkflowCreated').workflow
    const mcpNode = variant(await client.send(addWorkflowNodeRequest(mcpSession.id, mcpWorkflow.id, mcpAgent.id)), 'WorkflowNodeAdded').node
    await client.send(updateWorkflowNodeInstructionsRequest(
      mcpSession.id,
      mcpWorkflow.id,
      mcpNode.id,
      'Complete this publication drill workflow with the deterministic final output envelope emitted by the dev stub.',
    ))
    await client.send(setWorkflowNodeCanCompleteRunRequest(mcpSession.id, mcpWorkflow.id, mcpNode.id, true))
    const mcpEndpoint = variant(
      await client.send(createWorkflowEndpointRequest(mcpSession.id, mcpWorkflow.id, mcpNode.id, 'mcp')),
      'WorkflowEndpointCreated',
    ).endpoint
    const mcpPublication = variant(
      await client.send(createWorkflowPublicationRequest(mcpSession.id, mcpWorkflow.id, mcpEndpoint.id, {
        alias: 'public_mcp',
        route: '/mcp',
        methods: ['POST'],
        transport: { kind: 'mcp' },
        parser: { kind: 'json' },
        mode: 'sync',
      })),
      'WorkflowPublicationCreated',
    ).publication

    logStep('create_watchdog_session')
    const watchdogSession = variant(
      await client.send(createSessionRequest(watchdogWorkspace, watchdogWorkspace, 'publication-drill-watchdog')),
      'SessionCreated',
    ).session
    sessionIds.push(watchdogSession.id)
    await client.send(attachToSessionRequest(watchdogSession.id, `publication-drill-watchdog-${process.pid}`))
    const watchdogAgent = variant(
      await client.send(spawnAgentRequest(watchdogSession.id, 'dev-stub', 'watchdog-final', 'workflow-intermediate-node', watchdogWorkspace, 'low')),
      'AgentSpawned',
    ).agent
    const watchdogProviderRun = variant(
      await client.send(launchProviderRunRequest(watchdogSession.id, 'dev-stub', 'default', 'workflow-intermediate-node', 'low', watchdogAgent.id)),
      'ProviderRunLaunchAccepted',
    ).provider_run
    await waitForProviderRunReady(client, watchdogProviderRun.id)
    const watchdogWorkflow = variant(await client.send(createWorkflowRequest(watchdogSession.id, 'published-watchdog')), 'WorkflowCreated').workflow
    const watchdogNode = variant(await client.send(addWorkflowNodeRequest(watchdogSession.id, watchdogWorkflow.id, watchdogAgent.id)), 'WorkflowNodeAdded').node
    await client.send(updateWorkflowNodeInstructionsRequest(
      watchdogSession.id,
      watchdogWorkflow.id,
      watchdogNode.id,
      'Complete this publication drill workflow with the deterministic final output envelope emitted by the dev stub.',
    ))
    await client.send(setWorkflowNodeCanCompleteRunRequest(watchdogSession.id, watchdogWorkflow.id, watchdogNode.id, true))
    await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(watchdogSession.id, watchdogWorkflow.id, watchdogNode.id, true))
    const watchdogEndpoint = variant(
      await client.send(createWorkflowEndpointRequest(watchdogSession.id, watchdogWorkflow.id, watchdogNode.id, 'watchdog')),
      'WorkflowEndpointCreated',
    ).endpoint
    const watchdogPublication = variant(
      await client.send(createWorkflowPublicationRequest(watchdogSession.id, watchdogWorkflow.id, watchdogEndpoint.id, {
        alias: 'public_watchdog',
        route: '/watchdog',
        methods: ['POST'],
        transport: { kind: 'api_sse_json' },
        parser: { kind: 'json' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication

    if (process.env.ARROBA_PUBLICATION_LIVE_MANIFEST) {
      await runLivePublicationManifestMode({
        manifestPath: process.env.ARROBA_PUBLICATION_LIVE_MANIFEST,
        client,
        env,
        kernelUrl,
        relayPort,
        publications: [{
          key: 'human_http',
          transport: 'human_http',
          sessionId: browserSession.id,
          publication: humanHttpFinalPublication,
        }, {
          key: 'api_sse_json',
          transport: 'api_sse_json',
          sessionId: apiSseSession.id,
          publication: apiSseFinalPublication,
        }, {
          key: 'websocket_json',
          transport: 'websocket_json',
          sessionId: websocketSession.id,
          publication: websocketFinalPublication,
        }, {
          key: 'mcp',
          transport: 'mcp',
          sessionId: mcpSession.id,
          publication: mcpPublication,
        }, {
          key: 'watchdog',
          transport: 'watchdog',
          sessionId: watchdogSession.id,
          publication: watchdogPublication,
        }],
      })
      succeeded = true
      return
    }

    logStep('start_gateway', { publicationId: publication.id })
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: session.id,
        ARROBA_PUBLICATION_ID: publication.id,
      },
      'gateway',
    )
    await waitForGateway(gatewayUrl)
    const registeredPublication = await waitForRegisteredPublicationEndpoint(
      client,
      session.id,
      publication.id,
      `${gatewayUrl}/`,
      `http://127.0.0.1:${relayPort}/display/publication-`,
    )
    logStep('publication_endpoint_registered', {
      publicationId: registeredPublication.id,
      status: registeredPublication.status,
      openUrl: registeredPublication.open_url,
      deployment: registeredPublication.deployment?.kind ?? null,
    })
    if (registeredPublication.deployment?.kind !== 'tunnel') {
      throw new Error(`expected registered publication deployment tunnel, got ${JSON.stringify(registeredPublication.deployment)}`)
    }

    logStep('invoke_browser_html')
    const rootHtmlResponse = await fetch(`${gatewayUrl}/`, { headers: { accept: 'text/html' } })
    const rootHtml = await rootHtmlResponse.text()
    if (rootHtmlResponse.status !== 200 || !rootHtml.includes('invoke-form')) {
      throw new Error(`expected browser root form HTML, got ${rootHtmlResponse.status}: ${rootHtml.slice(0, 200)}`)
    }
    if (!rootHtml.includes('type="file" name="artifact" multiple')) {
      throw new Error(`expected browser root form artifact upload input, got: ${rootHtml.slice(0, 300)}`)
    }
    const browserResponse = await fetch(`${gatewayUrl}/qa/browser-publication`, { headers: { accept: 'text/html' } })
    const browserHtml = await browserResponse.text()
    if (browserResponse.status !== 200 || !browserHtml.includes('EventSource') || !browserHtml.includes("addEventListener('queued'")) {
      throw new Error(`expected browser invocation HTML with SSE subscription, got ${browserResponse.status}: ${browserHtml.slice(0, 200)}`)
    }

    logStep('invoke_browser_html_tunnel')
    const tunnelRootResponse = await fetch(registeredPublication.open_url, { headers: { accept: 'text/html' } })
    const tunnelRootHtml = await tunnelRootResponse.text()
    if (tunnelRootResponse.status !== 200 || !tunnelRootHtml.includes('invoke-form')) {
      throw new Error(`expected tunnel root form HTML, got ${tunnelRootResponse.status}: ${tunnelRootHtml.slice(0, 200)}`)
    }
    const tunnelBrowserUrl = new URL('qa/browser-publication-tunnel', registeredPublication.open_url).toString()
    const tunnelBrowserResponse = await fetch(tunnelBrowserUrl, { headers: { accept: 'text/html' } })
    const tunnelBrowserHtml = await tunnelBrowserResponse.text()
    if (tunnelBrowserResponse.status !== 200 || !tunnelBrowserHtml.includes('EventSource') || !tunnelBrowserHtml.includes("addEventListener('queued'")) {
      throw new Error(`expected tunnel browser invocation HTML with SSE subscription, got ${tunnelBrowserResponse.status}: ${tunnelBrowserHtml.slice(0, 200)}`)
    }

    logStep('invoke_browser_upload')
    const uploadResponse = await fetch(`${gatewayUrl}/.well-known/arroba/publication/human-http/invoke`, {
      method: 'POST',
      headers: { accept: 'text/html', 'content-type': 'application/json' },
      body: JSON.stringify({
        prompt: 'browser-upload-publication',
        artifacts: [{
          name: 'upload.txt',
          type: 'text/plain',
          size_bytes: 18,
          data_url: 'data:text/plain;base64,cHVibGljYXRpb24tdXBsb2Fk',
        }],
      }),
    })
    const uploadHtml = await uploadResponse.text()
    if (uploadResponse.status !== 200 || !uploadHtml.includes('EventSource') || !uploadHtml.includes("addEventListener('queued'")) {
      throw new Error(`expected browser upload invocation HTML with SSE subscription, got ${uploadResponse.status}: ${uploadHtml.slice(0, 200)}`)
    }

    logStep('invoke_http')
    const response = await fetch(`${gatewayUrl}/qa/ship-publication`)
    const body = await response.json()
    if (response.status !== 202) {
      throw new Error(`expected HTTP 202 from async publication, got ${response.status}: ${JSON.stringify(body)}`)
    }
    if (!body.accepted || !hasAcceptedRunMetadata(body)) {
      throw new Error(`gateway did not return accepted workflow run metadata: ${JSON.stringify(body)}`)
    }
    logStep('anonymous_ok', { publicationId: publication.id, workflowRunId: body.workflow_run?.id ?? null, queued: body.queued === true })

    logStep('parser_failure_400')
    const badParserResponse = await fetch(`${gatewayUrl}/qa/a/b`)
    if (badParserResponse.status !== 400) {
      throw new Error(`expected parser failure HTTP 400, got ${badParserResponse.status}: ${await badParserResponse.text()}`)
    }

    logStep('invoke_websocket')
    const webSocketAccepted = await invokePublicationWebSocket(
      `ws://127.0.0.1:${gatewayPort}/.well-known/arroba/publication/ws`,
      { task: 'websocket-publication' },
    )
    logStep('websocket_ok', { workflowRunId: webSocketAccepted.accepted.workflow_run?.id ?? null, queued: webSocketAccepted.accepted.queued === true })

    await stopProcess(gateway)
    gateway = null

    logStep('invoke_api_sse')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: session.id,
        ARROBA_PUBLICATION_ID: apiSsePublication.id,
      },
      'gateway-api-sse',
    )
    await waitForGateway(gatewayUrl)
    const apiSseResponse = await fetch(`${gatewayUrl}/invoke`, {
      method: 'POST',
      headers: { accept: 'text/event-stream', 'content-type': 'application/json' },
      body: JSON.stringify({
        prompt: 'api-sse-publication',
        artifacts: [{
          name: 'input.txt',
          type: 'text/plain',
          base64: 'YXBpLXN0cmVhbQ==',
        }, {
          name: 'input-url.txt',
          type: 'text/plain',
          url: 'https://example.invalid/arroba-publication-input.txt',
        }],
      }),
    })
    const apiSseBody = await apiSseResponse.text()
    if (apiSseResponse.status !== 200 || !apiSseBody.includes('event: queued')) {
      throw new Error(`expected API SSE queued event, got ${apiSseResponse.status}: ${apiSseBody.slice(0, 400)}`)
    }
    logStep('api_sse_queued_ok')
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_api_sse_final')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: apiSseSession.id,
        ARROBA_PUBLICATION_ID: apiSseFinalPublication.id,
      },
      'gateway-api-sse-final',
    )
    await waitForGateway(gatewayUrl)
    const apiSseFinalResponse = await fetch(`${gatewayUrl}/invoke`, {
      method: 'POST',
      headers: { accept: 'text/event-stream', 'content-type': 'application/json' },
      body: JSON.stringify({ prompt: 'api-sse-final-publication' }),
    })
    const apiSseFinalBody = await apiSseFinalResponse.text()
    const apiSseFinalEvents = sseEventNames(apiSseFinalBody)
    if (
      apiSseFinalResponse.status !== 200
      || !apiSseFinalEvents.includes('queued')
      || !apiSseFinalEvents.includes('started')
      || !apiSseFinalEvents.includes('partial')
      || !apiSseFinalEvents.includes('final')
      || (!apiSseFinalBody.includes('"value":1841') && !apiSseFinalBody.includes('\\"value\\":1841'))
      || (!apiSseFinalBody.includes('"value":1842') && !apiSseFinalBody.includes('\\"value\\":1842'))
    ) {
      const errorIndex = apiSseFinalBody.lastIndexOf('event: error')
      const diagnostic = errorIndex >= 0 ? apiSseFinalBody.slice(errorIndex, errorIndex + 800) : apiSseFinalBody.slice(0, 2_000)
      throw new Error(`expected API SSE queued/started/partial/final with deterministic output, got ${apiSseFinalResponse.status} ${JSON.stringify(apiSseFinalEvents)}: ${diagnostic}`)
    }
    logStep('api_sse_final_ok', { events: apiSseFinalEvents })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_api_sse_final_tunnel')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: apiSseTunnel.session.id,
        ARROBA_PUBLICATION_ID: apiSseTunnel.publication.id,
      },
      'gateway-api-sse-final-tunnel',
    )
    await waitForGateway(gatewayUrl)
    const registeredApiSseFinalPublication = await waitForRegisteredPublicationEndpoint(
      client,
      apiSseTunnel.session.id,
      apiSseTunnel.publication.id,
      `${gatewayUrl}/`,
      `http://127.0.0.1:${relayPort}/display/publication-`,
    )
    const apiSseFinalTunnelUrl = new URL('invoke', registeredApiSseFinalPublication.open_url).toString()
    const apiSseFinalTunnelResponse = await fetch(apiSseFinalTunnelUrl, {
      method: 'POST',
      headers: { accept: 'text/event-stream', 'content-type': 'application/json' },
      body: JSON.stringify({ prompt: 'api-sse-final-publication-tunnel' }),
    })
    const apiSseFinalTunnelBody = await apiSseFinalTunnelResponse.text()
    const apiSseFinalTunnelEvents = sseEventNames(apiSseFinalTunnelBody)
    if (
      apiSseFinalTunnelResponse.status !== 200
      || !apiSseFinalTunnelEvents.includes('queued')
      || !apiSseFinalTunnelEvents.includes('started')
      || !apiSseFinalTunnelEvents.includes('partial')
      || !apiSseFinalTunnelEvents.includes('final')
      || (!apiSseFinalTunnelBody.includes('"value":1841') && !apiSseFinalTunnelBody.includes('\\"value\\":1841'))
      || (!apiSseFinalTunnelBody.includes('"value":1842') && !apiSseFinalTunnelBody.includes('\\"value\\":1842'))
    ) {
      const errorIndex = apiSseFinalTunnelBody.lastIndexOf('event: error')
      const diagnostic = errorIndex >= 0 ? apiSseFinalTunnelBody.slice(errorIndex, errorIndex + 800) : apiSseFinalTunnelBody.slice(0, 2_000)
      throw new Error(`expected tunnel API SSE queued/started/partial/final with deterministic output, got ${apiSseFinalTunnelResponse.status} ${JSON.stringify(apiSseFinalTunnelEvents)}: ${diagnostic}`)
    }
    logStep('api_sse_final_tunnel_ok', { events: apiSseFinalTunnelEvents, openUrl: registeredApiSseFinalPublication.open_url })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_websocket_final')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: websocketSession.id,
        ARROBA_PUBLICATION_ID: websocketFinalPublication.id,
      },
      'gateway-websocket-final',
    )
    await waitForGateway(gatewayUrl)
    const webSocketFinal = await invokePublicationWebSocket(
      `ws://127.0.0.1:${gatewayPort}/.well-known/arroba/publication/ws`,
      { prompt: 'websocket-final-publication' },
      { waitForFinal: true },
    )
    const webSocketFinalTypes = webSocketFinal.messages.map((message) => message.type)
    const webSocketFinalBody = JSON.stringify(webSocketFinal.messages)
    if (
      !webSocketFinalTypes.includes('accepted')
      || !webSocketFinalTypes.includes('queued')
      || !webSocketFinalTypes.includes('started')
      || !webSocketFinalTypes.includes('partial')
      || !webSocketFinalTypes.includes('final')
      || (!webSocketFinalBody.includes('"value":1841') && !webSocketFinalBody.includes('\\"value\\":1841'))
      || (!webSocketFinalBody.includes('"value":1842') && !webSocketFinalBody.includes('\\"value\\":1842'))
    ) {
      throw new Error(`expected websocket accepted/queued/started/partial/final with deterministic output, got ${JSON.stringify(webSocketFinal.messages)}`)
    }
    logStep('websocket_final_ok', { events: webSocketFinalTypes })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_websocket_final_tunnel')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: websocketTunnel.session.id,
        ARROBA_PUBLICATION_ID: websocketTunnel.publication.id,
      },
      'gateway-websocket-final-tunnel',
    )
    await waitForGateway(gatewayUrl)
    const registeredWebSocketPublication = await waitForRegisteredPublicationEndpoint(
      client,
      websocketTunnel.session.id,
      websocketTunnel.publication.id,
      `${gatewayUrl}/`,
      `http://127.0.0.1:${relayPort}/display/publication-`,
    )
    const webSocketTunnelUrl = websocketUrlFromHttp(
      new URL('.well-known/arroba/publication/ws', registeredWebSocketPublication.open_url).toString(),
    )
    const webSocketTunnelFinal = await invokePublicationWebSocket(
      webSocketTunnelUrl,
      { prompt: 'websocket-final-publication-tunnel' },
      { waitForFinal: true },
    )
    const webSocketTunnelTypes = webSocketTunnelFinal.messages.map((message) => message.type)
    const webSocketTunnelBody = JSON.stringify(webSocketTunnelFinal.messages)
    if (
      !webSocketTunnelTypes.includes('accepted')
      || !webSocketTunnelTypes.includes('queued')
      || !webSocketTunnelTypes.includes('started')
      || !webSocketTunnelTypes.includes('partial')
      || !webSocketTunnelTypes.includes('final')
      || (!webSocketTunnelBody.includes('"value":1841') && !webSocketTunnelBody.includes('\\"value\\":1841'))
      || (!webSocketTunnelBody.includes('"value":1842') && !webSocketTunnelBody.includes('\\"value\\":1842'))
    ) {
      throw new Error(`expected tunnel websocket accepted/queued/started/partial/final with deterministic output, got ${JSON.stringify(webSocketTunnelFinal.messages)}`)
    }
    logStep('websocket_final_tunnel_ok', { events: webSocketTunnelTypes, openUrl: registeredWebSocketPublication.open_url })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_human_http_browser_final')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: browserSession.id,
        ARROBA_PUBLICATION_ID: humanHttpFinalPublication.id,
      },
      'gateway-human-http-final',
    )
    await waitForGateway(gatewayUrl)
    await runHumanHttpBrowserDrill({
      url: `${gatewayUrl}/final/browser-final-publication`,
      root,
    })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_human_http_browser_root_form')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: browserRoot.session.id,
        ARROBA_PUBLICATION_ID: browserRoot.publication.id,
      },
      'gateway-human-http-root-form',
    )
    await waitForGateway(gatewayUrl)
    await runHumanHttpRootFormBrowserDrill({
      baseUrl: `${gatewayUrl}/`,
      root,
      timeoutMs: 90_000,
    })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_mcp')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: mcpSession.id,
        ARROBA_PUBLICATION_ID: mcpPublication.id,
      },
      'gateway-mcp',
    )
    await waitForGateway(gatewayUrl)
    const mcpToolsResponse = await fetch(`${gatewayUrl}/mcp`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list' }),
    })
    const mcpToolsBody = await mcpToolsResponse.json()
    const mcpToolName = mcpToolsBody.result?.tools?.[0]?.name
    if (mcpToolsResponse.status !== 200 || typeof mcpToolName !== 'string') {
      throw new Error(`expected MCP tools/list to expose publication tool, got ${mcpToolsResponse.status}: ${JSON.stringify(mcpToolsBody)}`)
    }
    const mcpCallResponse = await fetch(`${gatewayUrl}/mcp`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: 2,
        method: 'tools/call',
        params: { name: mcpToolName, arguments: { prompt: 'mcp-publication' } },
      }),
    })
    const mcpCallBody = await mcpCallResponse.json()
    const mcpText = mcpCallBody.result?.content?.[0]?.text ?? ''
    if (
      mcpCallResponse.status !== 200
      || mcpCallBody.result?.isError !== false
      || (!mcpText.includes('"value":1842') && !mcpText.includes('\\"value\\":1842'))
    ) {
      throw new Error(`expected MCP tools/call final output, got ${mcpCallResponse.status}: ${JSON.stringify(mcpCallBody).slice(0, 1_200)}`)
    }
    logStep('mcp_ok', { tool: mcpToolName, workflowRunId: mcpCallBody.result?.structuredContent?.workflow_run_id ?? null })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_https')
    const previousTlsReject = process.env.NODE_TLS_REJECT_UNAUTHORIZED
    process.env.NODE_TLS_REJECT_UNAUTHORIZED = '0'
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayHttpsPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: session.id,
        ARROBA_PUBLICATION_ID: publication.id,
        ARROBA_PUBLICATION_TLS_KEY_FILE: tls.keyFile,
        ARROBA_PUBLICATION_TLS_CERT_FILE: tls.certFile,
      },
      'gateway-https',
    )
    let httpsBody = null
    let httpsResponse = null
    try {
      await waitForGateway(gatewayHttpsUrl)
      httpsResponse = await fetch(`${gatewayHttpsUrl}/qa/ship-publication-secure`)
      httpsBody = await httpsResponse.json()
      if (httpsResponse.status !== 202 || !hasAcceptedRunMetadata(httpsBody)) {
        throw new Error(`expected HTTPS 202 from async publication, got ${httpsResponse.status}: ${JSON.stringify(httpsBody)}`)
      }
      logStep('invoke_wss')
      const wssAccepted = await invokePublicationWebSocket(
        `wss://127.0.0.1:${gatewayHttpsPort}/.well-known/arroba/publication/ws`,
        { task: 'wss-publication' },
        { rejectUnauthorized: false },
      )
      logStep('wss_ok', { workflowRunId: wssAccepted.accepted.workflow_run?.id ?? null, queued: wssAccepted.accepted.queued === true })
    } finally {
      if (previousTlsReject === undefined) delete process.env.NODE_TLS_REJECT_UNAUTHORIZED
      else process.env.NODE_TLS_REJECT_UNAUTHORIZED = previousTlsReject
    }
    await stopProcess(gateway)
    gateway = null

    logStep('export_publication_package')
    const exportDir = path.join(root, 'exported-publication')
    const exportResult = await executeShellCommand(
      parseShellCommand(`workflow publication export ${publication.id} ${exportDir} --kernel-url ${kernelUrl}`),
      createDefaultShellContext({
        workspace,
        worktree: workspace,
        sessionId: session.id,
        workflowId: workflow.id,
      }),
      { client },
    )
    if (!exportResult.ok) {
      throw new Error(`publication export failed: ${exportResult.message}`)
    }
    await assertPackageDoesNotContain(exportDir, [
      relayToken,
    ])
    gateway = startProcess(
      cliBinary,
      ['serve', exportDir, String(gatewayPort), '--kernel-url', kernelUrl],
      {
        ...env,
        HOST: '127.0.0.1',
      },
      'arroba-serve-exported',
    )
    await waitForGateway(gatewayUrl)
    const exportedStatusResponse = await fetch(`${gatewayUrl}/.well-known/arroba/publication/status`)
    const exportedStatusBody = await exportedStatusResponse.json()
    const exportedRuntimeSessionId = exportedStatusBody.runtime_session_id
    if (exportedStatusResponse.status !== 200 || typeof exportedRuntimeSessionId !== 'string') {
      throw new Error(`expected exported publication status with runtime session id, got ${exportedStatusResponse.status}: ${JSON.stringify(exportedStatusBody)}`)
    }
    sessionIds.push(exportedRuntimeSessionId)
    await assertPublicationRuntimeSessionHidden(client, exportedRuntimeSessionId)
    const exportedResponse = await fetch(`${gatewayUrl}/qa/exported-publication`)
    const exportedBody = await exportedResponse.json()
    if (exportedResponse.status !== 202 || !hasAcceptedRunMetadata(exportedBody)) {
      throw new Error(`expected exported package HTTP 202, got ${exportedResponse.status}: ${JSON.stringify(exportedBody)}`)
    }
    await stopProcess(gateway)
    gateway = null
    await client.send(endSessionRequest(exportedRuntimeSessionId)).catch(() => {})

    logStep('invoke_ipc_exported')
    const ipcResult = await run(process.execPath, [
      path.join(repoRoot, 'apps/server/dist/workflow-call.js'),
      '--package',
      path.join(exportDir, 'publication.json'),
      '--input',
      JSON.stringify({ task: 'ipc-exported-publication' }),
      '--mode',
      'async',
    ], { env })
    if (ipcResult.code !== 0) {
      throw new Error(`expected IPC exported invocation to succeed\nstdout:\n${ipcResult.stdout}\nstderr:\n${ipcResult.stderr}`)
    }
    const ipcBody = JSON.parse(ipcResult.stdout)
    if (!ipcBody.accepted || !hasAcceptedRunMetadata(ipcBody)) {
      throw new Error(`expected IPC accepted run metadata, got ${ipcResult.stdout}`)
    }
    if (typeof ipcBody.runtime_session_id !== 'string') {
      throw new Error(`expected IPC output to include runtime_session_id, got ${ipcResult.stdout}`)
    }
    sessionIds.push(ipcBody.runtime_session_id)
    await assertPublicationRuntimeSessionHidden(client, ipcBody.runtime_session_id)
    await client.send(endSessionRequest(ipcBody.runtime_session_id)).catch(() => {})

    logStep('serve_provider_override_prompt')
    const overrideExportDir = path.join(root, 'exported-publication-provider-override')
    const overridePackage = await createUnavailableProviderPackage(exportDir, overrideExportDir)
    gateway = startServeWithProviderPrompt({
      cliBinary,
      packageDir: overrideExportDir,
      port: gatewayPort,
      kernelUrl,
      env: {
        ...env,
        HOST: '127.0.0.1',
      },
      provider: 'dev-stub',
      model: 'publication-drill-model',
      effort: 'low',
    })
    await waitForGateway(gatewayUrl)
    const overrideStatusResponse = await fetch(`${gatewayUrl}/.well-known/arroba/publication/status`)
    const overrideStatusBody = await overrideStatusResponse.json()
    const overrideRuntimeSessionId = overrideStatusBody.runtime_session_id
    if (overrideStatusResponse.status !== 200 || typeof overrideRuntimeSessionId !== 'string') {
      throw new Error(`expected override publication status with runtime session id, got ${overrideStatusResponse.status}: ${JSON.stringify(overrideStatusBody)}`)
    }
    sessionIds.push(overrideRuntimeSessionId)
    await assertPublicationRuntimeSessionHidden(client, overrideRuntimeSessionId)
    const overrideBindings = JSON.parse(await readFile(path.join(overrideExportDir, 'bindings.local.json'), 'utf8'))
    const overrideReplacement = overrideBindings.provider_model_overrides
      ?.find((candidate) => candidate.agent_id === overridePackage.agentId)
      ?.replacement
    if (
      overrideReplacement?.provider !== 'dev-stub'
      || overrideReplacement?.model !== 'publication-drill-model'
      || overrideReplacement?.effort !== 'low'
    ) {
      throw new Error(`expected provider override to persist dev-stub/publication-drill-model/low, got ${JSON.stringify(overrideBindings)}`)
    }
    const overrideResponse = await fetch(`${gatewayUrl}/qa/provider-override-publication`)
    const overrideBody = await overrideResponse.json()
    if (overrideResponse.status !== 202 || !hasAcceptedRunMetadata(overrideBody)) {
      throw new Error(`expected provider override package HTTP 202, got ${overrideResponse.status}: ${JSON.stringify(overrideBody)}`)
    }
    if (
      !gateway.logs.stdout.includes('Replacement provider:')
      || !gateway.logs.stdout.includes('Replacement model')
      || !gateway.logs.stdout.includes('Replacement effort')
    ) {
      throw new Error(`expected provider override prompt transcript, got stdout:\n${gateway.logs.stdout}\nstderr:\n${gateway.logs.stderr}`)
    }
    await stopProcess(gateway)
    gateway = null
    await client.send(endSessionRequest(overrideRuntimeSessionId)).catch(() => {})
    logStep('serve_provider_override_prompt_ok', { agentId: overridePackage.agentId })
    await client.send(endSessionRequest(session.id)).catch(() => {})

    logStep('watchdog_publication_export')
    const watchdog = variant(
      await client.send(createWorkflowWatchdogRequest(
        watchdogSession.id,
        watchdogWorkflow.id,
        watchdogEndpoint.id,
        60,
        'watchdog-publication',
        'queue',
        1,
      )),
      'WorkflowWatchdogCreated',
    ).watchdog
    const watchdogExportDir = path.join(root, 'exported-watchdog-publication')
    const watchdogExportResult = await executeShellCommand(
      parseShellCommand(`workflow publication export ${watchdogPublication.id} ${watchdogExportDir} --kernel-url ${kernelUrl}`),
      createDefaultShellContext({
        workspace: watchdogWorkspace,
        worktree: watchdogWorkspace,
        sessionId: watchdogSession.id,
        workflowId: watchdogWorkflow.id,
      }),
      { client },
    )
    if (!watchdogExportResult.ok) {
      throw new Error(`watchdog publication export failed: ${watchdogExportResult.message}`)
    }
    const watchdogSnapshot = JSON.parse(await readFile(path.join(watchdogExportDir, 'workflow.snapshot.json'), 'utf8'))
    if (watchdogSnapshot.watchdogs?.[0]?.id !== watchdog.id) {
      throw new Error(`expected exported watchdog ${watchdog.id}, got ${JSON.stringify(watchdogSnapshot.watchdogs)}`)
    }
    watchdogSnapshot.watchdogs[0].next_run_at_ms = 0
    await writeFile(path.join(watchdogExportDir, 'workflow.snapshot.json'), `${JSON.stringify(watchdogSnapshot, null, 2)}\n`)
    gateway = startProcess(
      cliBinary,
      ['serve', watchdogExportDir, String(gatewayPort), '--kernel-url', kernelUrl],
      {
        ...env,
        HOST: '127.0.0.1',
      },
      'arroba-serve-watchdog',
    )
    await waitForGateway(gatewayUrl)
    const statusResponse = await fetch(`${gatewayUrl}/.well-known/arroba/publication/status`)
    const statusBody = await statusResponse.json()
    const runtimeSessionId = statusBody.runtime_session_id
    if (statusResponse.status !== 200 || typeof runtimeSessionId !== 'string') {
      throw new Error(`expected publication status with runtime session id, got ${statusResponse.status}: ${JSON.stringify(statusBody)}`)
    }
    if (statusBody.watchdog_count !== 1 || statusBody.watchdogs?.[0]?.id !== watchdog.id) {
      throw new Error(`expected publication status to expose watchdog ${watchdog.id}, got ${JSON.stringify(statusBody)}`)
    }
    sessionIds.push(runtimeSessionId)
    await assertPublicationRuntimeSessionHidden(client, runtimeSessionId)
    const watchdogRuntimeSession = variant(
      await client.send(getSessionStateRequest(runtimeSessionId)),
      'SessionState',
    ).session
    if (watchdogRuntimeSession.workspace_id !== watchdogWorkspace || watchdogRuntimeSession.worktree_id !== watchdogWorkspace) {
      throw new Error(`expected watchdog runtime workspace ${watchdogWorkspace}, got ${JSON.stringify({
        workspace_id: watchdogRuntimeSession.workspace_id,
        worktree_id: watchdogRuntimeSession.worktree_id,
      })}`)
    }
    const watchdogRun = await waitForWatchdogWorkflowRun(client, runtimeSessionId, watchdogWorkflow.id, { requireOutput: true })
    const statusAfterRun = await waitForPublicationStatusLatestOutput(gatewayUrl, watchdogRun.final_output?.message)
    logStep('watchdog_publication_ok', {
      runtimeSessionId,
      workflowRunId: watchdogRun.id,
      status: watchdogRun.status,
      latestOutput: statusAfterRun.latest_output?.message,
    })
    await stopProcess(gateway)
    gateway = null

    if (process.env.ARROBA_PUBLICATION_CONTAINER_DRILL === '1') {
      logStep('container_publication_build')
      const publicationContainerImage = `arroba-publication-drill:${process.pid}`
      dockerImages.push(publicationContainerImage)
      await buildPublicationContainerImage(publicationContainerImage)

      logStep('container_human_http_export')
      const humanHttpContainerExportDir = path.join(root, 'container-human-http-package')
      const humanHttpContainerPackageDir = path.join(root, 'container-human-http-portable')
      const humanHttpContainerExportResult = await executeShellCommand(
        parseShellCommand(`workflow publication export ${humanHttpFinalPublication.id} ${humanHttpContainerExportDir} --kernel-url ${kernelUrl}`),
        createDefaultShellContext({
          workspace: browserWorkspace,
          worktree: browserWorkspace,
          sessionId: browserSession.id,
          workflowId: browserWorkflow.id,
        }),
        { client },
      )
      if (!humanHttpContainerExportResult.ok) {
        throw new Error(`human_http container publication export failed: ${humanHttpContainerExportResult.message}`)
      }
      await createContainerPortablePackage(humanHttpContainerExportDir, humanHttpContainerPackageDir)
      const containerHumanPort = await freePort()
      const containerHumanUrl = `http://127.0.0.1:${containerHumanPort}`
      const containerHumanName = `arroba-publication-human-${process.pid}`
      dockerContainers.push(containerHumanName)
      let containerHumanPromptRuntimeSessionId = null
      let containerProcess = startPublicationContainer({
        image: publicationContainerImage,
        name: containerHumanName,
        packageDir: humanHttpContainerPackageDir,
        workspaceDir: browserWorkspace,
        port: containerHumanPort,
      })
      try {
        await waitForContainerGateway(containerHumanUrl, containerProcess, 60_000)
        const containerStatusResponse = await fetch(`${containerHumanUrl}/.well-known/arroba/publication/status`)
        const containerStatusBody = await containerStatusResponse.json()
        if (containerStatusResponse.status !== 200 || typeof containerStatusBody.runtime_session_id !== 'string') {
          throw new Error(`expected container human_http status with runtime session id, got ${containerStatusResponse.status}: ${JSON.stringify(containerStatusBody)}`)
        }
        containerHumanPromptRuntimeSessionId = containerStatusBody.runtime_session_id
        const containerHumanResponse = await fetch(`${containerHumanUrl}/`, {
          headers: { accept: 'text/html' },
        })
        const containerHumanBody = await containerHumanResponse.text()
        if (
          containerHumanResponse.status !== 200
          || !containerHumanBody.includes('invoke-form')
          || !containerHumanBody.includes('type="file" name="artifact" multiple')
          || !containerHumanBody.includes('/final/')
        ) {
          throw new Error(`expected container human_http root form with prompt and artifact upload, got ${containerHumanResponse.status}: ${containerHumanBody.slice(0, 1_000)}`)
        }
        await runHumanHttpBrowserDrill({
          url: `${containerHumanUrl}/final/container-human-http-browser`,
          root,
          timeoutMs: 90_000,
        })
        logStep('container_human_http_prompt_url_ok', { runtimeSessionId: containerStatusBody.runtime_session_id })
      } finally {
        await stopProcess(containerProcess)
        await removeDockerContainer(containerHumanName).catch(() => {})
        containerProcess = null
      }
      const containerHumanRootPort = await freePort()
      const containerHumanRootUrl = `http://127.0.0.1:${containerHumanRootPort}`
      const containerHumanRootName = `arroba-publication-human-root-${process.pid}`
      dockerContainers.push(containerHumanRootName)
      containerProcess = startPublicationContainer({
        image: publicationContainerImage,
        name: containerHumanRootName,
        packageDir: humanHttpContainerPackageDir,
        workspaceDir: browserWorkspace,
        port: containerHumanRootPort,
      })
      try {
        await waitForContainerGateway(containerHumanRootUrl, containerProcess, 60_000)
        const containerStatusResponse = await fetch(`${containerHumanRootUrl}/.well-known/arroba/publication/status`)
        const containerStatusBody = await containerStatusResponse.json()
        if (containerStatusResponse.status !== 200 || typeof containerStatusBody.runtime_session_id !== 'string') {
          throw new Error(`expected container human_http root status with runtime session id, got ${containerStatusResponse.status}: ${JSON.stringify(containerStatusBody)}`)
        }
        await runHumanHttpRootFormBrowserDrill({
          baseUrl: `${containerHumanRootUrl}/`,
          root,
          timeoutMs: 90_000,
        })
        logStep('container_human_http_ok', {
          promptRuntimeSessionId: containerHumanPromptRuntimeSessionId,
          rootFormRuntimeSessionId: containerStatusBody.runtime_session_id,
        })
      } finally {
        await stopProcess(containerProcess)
        await removeDockerContainer(containerHumanRootName).catch(() => {})
        containerProcess = null
      }

      logStep('container_api_sse_export')
      const apiSseContainerExportDir = path.join(root, 'container-api-sse-package')
      const apiSseContainerPackageDir = path.join(root, 'container-api-sse-portable')
      const apiSseContainerExportResult = await executeShellCommand(
        parseShellCommand(`workflow publication export ${apiSseFinalPublication.id} ${apiSseContainerExportDir} --kernel-url ${kernelUrl}`),
        createDefaultShellContext({
          workspace: apiSseWorkspace,
          worktree: apiSseWorkspace,
          sessionId: apiSseSession.id,
          workflowId: apiSseWorkflow.id,
        }),
        { client },
      )
      if (!apiSseContainerExportResult.ok) {
        throw new Error(`api_sse_json container publication export failed: ${apiSseContainerExportResult.message}`)
      }
      await createContainerPortablePackage(apiSseContainerExportDir, apiSseContainerPackageDir)
      const containerApiPort = await freePort()
      const containerApiUrl = `http://127.0.0.1:${containerApiPort}`
      const containerApiName = `arroba-publication-api-${process.pid}`
      dockerContainers.push(containerApiName)
      containerProcess = startPublicationContainer({
        image: publicationContainerImage,
        name: containerApiName,
        packageDir: apiSseContainerPackageDir,
        workspaceDir: apiSseWorkspace,
        port: containerApiPort,
      })
      try {
        await waitForContainerGateway(containerApiUrl, containerProcess, 60_000)
        const containerApiStatusResponse = await fetch(`${containerApiUrl}/.well-known/arroba/publication/status`)
        const containerApiStatusBody = await containerApiStatusResponse.json()
        if (containerApiStatusResponse.status !== 200 || typeof containerApiStatusBody.runtime_session_id !== 'string') {
          throw new Error(`expected container api_sse_json status with runtime session id, got ${containerApiStatusResponse.status}: ${JSON.stringify(containerApiStatusBody)}`)
        }
        const containerApiResponse = await fetch(`${containerApiUrl}/invoke`, {
          method: 'POST',
          headers: { accept: 'text/event-stream', 'content-type': 'application/json' },
          body: JSON.stringify({ prompt: 'container-api-sse-publication' }),
        })
        const containerApiBody = await containerApiResponse.text()
        const containerApiEvents = sseEventNames(containerApiBody)
        if (
          containerApiResponse.status !== 200
          || !containerApiEvents.includes('queued')
          || !containerApiEvents.includes('started')
          || !containerApiEvents.includes('partial')
          || !containerApiEvents.includes('final')
          || (!containerApiBody.includes('"value":1841') && !containerApiBody.includes('\\"value\\":1841'))
          || (!containerApiBody.includes('"value":1842') && !containerApiBody.includes('\\"value\\":1842'))
        ) {
          throw new Error(`expected container API SSE queued/started/partial/final with deterministic output, got ${containerApiResponse.status} ${JSON.stringify(containerApiEvents)}: ${containerApiBody.slice(0, 2_000)}`)
        }
        logStep('container_api_sse_ok', { runtimeSessionId: containerApiStatusBody.runtime_session_id, events: containerApiEvents })
      } finally {
        await stopProcess(containerProcess)
        await removeDockerContainer(containerApiName).catch(() => {})
        containerProcess = null
      }

      logStep('container_websocket_export')
      const websocketContainerExportDir = path.join(root, 'container-websocket-package')
      const websocketContainerPackageDir = path.join(root, 'container-websocket-portable')
      const websocketContainerExportResult = await executeShellCommand(
        parseShellCommand(`workflow publication export ${websocketFinalPublication.id} ${websocketContainerExportDir} --kernel-url ${kernelUrl}`),
        createDefaultShellContext({
          workspace: websocketWorkspace,
          worktree: websocketWorkspace,
          sessionId: websocketSession.id,
          workflowId: websocketWorkflow.id,
        }),
        { client },
      )
      if (!websocketContainerExportResult.ok) {
        throw new Error(`websocket_json container publication export failed: ${websocketContainerExportResult.message}`)
      }
      await createContainerPortablePackage(websocketContainerExportDir, websocketContainerPackageDir)
      const containerWebSocketPort = await freePort()
      const containerWebSocketUrl = `http://127.0.0.1:${containerWebSocketPort}`
      const containerWebSocketName = `arroba-publication-websocket-${process.pid}`
      dockerContainers.push(containerWebSocketName)
      containerProcess = startPublicationContainer({
        image: publicationContainerImage,
        name: containerWebSocketName,
        packageDir: websocketContainerPackageDir,
        workspaceDir: websocketWorkspace,
        port: containerWebSocketPort,
      })
      try {
        await waitForContainerGateway(containerWebSocketUrl, containerProcess, 60_000)
        const containerWebSocketStatusResponse = await fetch(`${containerWebSocketUrl}/.well-known/arroba/publication/status`)
        const containerWebSocketStatusBody = await containerWebSocketStatusResponse.json()
        if (containerWebSocketStatusResponse.status !== 200 || typeof containerWebSocketStatusBody.runtime_session_id !== 'string') {
          throw new Error(`expected container websocket_json status with runtime session id, got ${containerWebSocketStatusResponse.status}: ${JSON.stringify(containerWebSocketStatusBody)}`)
        }
        const containerWebSocket = await invokePublicationWebSocket(
          `ws://127.0.0.1:${containerWebSocketPort}/.well-known/arroba/publication/ws`,
          { prompt: 'container-websocket-publication' },
          { waitForFinal: true },
        )
        const containerWebSocketTypes = containerWebSocket.messages.map((message) => message.type)
        const containerWebSocketBody = JSON.stringify(containerWebSocket.messages)
        if (
          !containerWebSocketTypes.includes('accepted')
          || !containerWebSocketTypes.includes('queued')
          || !containerWebSocketTypes.includes('started')
          || !containerWebSocketTypes.includes('partial')
          || !containerWebSocketTypes.includes('final')
          || (!containerWebSocketBody.includes('"value":1841') && !containerWebSocketBody.includes('\\"value\\":1841'))
          || (!containerWebSocketBody.includes('"value":1842') && !containerWebSocketBody.includes('\\"value\\":1842'))
        ) {
          throw new Error(`expected container websocket_json accepted/queued/started/partial/final with deterministic output, got ${JSON.stringify(containerWebSocket.messages)}`)
        }
        logStep('container_websocket_ok', { runtimeSessionId: containerWebSocketStatusBody.runtime_session_id, events: containerWebSocketTypes })
      } finally {
        await stopProcess(containerProcess)
        await removeDockerContainer(containerWebSocketName).catch(() => {})
        containerProcess = null
      }

      logStep('container_mcp_export')
      const mcpContainerExportDir = path.join(root, 'container-mcp-package')
      const mcpContainerPackageDir = path.join(root, 'container-mcp-portable')
      const mcpContainerExportResult = await executeShellCommand(
        parseShellCommand(`workflow publication export ${mcpPublication.id} ${mcpContainerExportDir} --kernel-url ${kernelUrl}`),
        createDefaultShellContext({
          workspace: mcpWorkspace,
          worktree: mcpWorkspace,
          sessionId: mcpSession.id,
          workflowId: mcpWorkflow.id,
        }),
        { client },
      )
      if (!mcpContainerExportResult.ok) {
        throw new Error(`mcp container publication export failed: ${mcpContainerExportResult.message}`)
      }
      await createContainerPortablePackage(mcpContainerExportDir, mcpContainerPackageDir)
      const containerMcpPort = await freePort()
      const containerMcpUrl = `http://127.0.0.1:${containerMcpPort}`
      const containerMcpName = `arroba-publication-mcp-${process.pid}`
      dockerContainers.push(containerMcpName)
      containerProcess = startPublicationContainer({
        image: publicationContainerImage,
        name: containerMcpName,
        packageDir: mcpContainerPackageDir,
        workspaceDir: mcpWorkspace,
        port: containerMcpPort,
      })
      try {
        await waitForContainerGateway(containerMcpUrl, containerProcess, 60_000)
        const containerMcpStatusResponse = await fetch(`${containerMcpUrl}/.well-known/arroba/publication/status`)
        const containerMcpStatusBody = await containerMcpStatusResponse.json()
        if (containerMcpStatusResponse.status !== 200 || typeof containerMcpStatusBody.runtime_session_id !== 'string') {
          throw new Error(`expected container mcp status with runtime session id, got ${containerMcpStatusResponse.status}: ${JSON.stringify(containerMcpStatusBody)}`)
        }
        const containerMcpToolsResponse = await fetch(`${containerMcpUrl}/mcp`, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list' }),
        })
        const containerMcpToolsBody = await containerMcpToolsResponse.json()
        const containerMcpToolName = containerMcpToolsBody.result?.tools?.[0]?.name
        if (containerMcpToolsResponse.status !== 200 || typeof containerMcpToolName !== 'string') {
          throw new Error(`expected container MCP tools/list to expose publication tool, got ${containerMcpToolsResponse.status}: ${JSON.stringify(containerMcpToolsBody)}`)
        }
        const containerMcpCallResponse = await fetch(`${containerMcpUrl}/mcp`, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({
            jsonrpc: '2.0',
            id: 2,
            method: 'tools/call',
            params: { name: containerMcpToolName, arguments: { prompt: 'container-mcp-publication' } },
          }),
        })
        const containerMcpCallBody = await containerMcpCallResponse.json()
        const containerMcpText = containerMcpCallBody.result?.content?.[0]?.text ?? ''
        if (
          containerMcpCallResponse.status !== 200
          || containerMcpCallBody.result?.isError !== false
          || (!containerMcpText.includes('"value":1842') && !containerMcpText.includes('\\"value\\":1842'))
        ) {
          throw new Error(`expected container MCP tools/call final output, got ${containerMcpCallResponse.status}: ${JSON.stringify(containerMcpCallBody).slice(0, 1_200)}`)
        }
        logStep('container_mcp_ok', { runtimeSessionId: containerMcpStatusBody.runtime_session_id, tool: containerMcpToolName })
      } finally {
        await stopProcess(containerProcess)
        await removeDockerContainer(containerMcpName).catch(() => {})
        containerProcess = null
      }

      logStep('container_watchdog_export')
      const watchdogContainerPackageDir = path.join(root, 'container-watchdog-portable')
      await createContainerPortablePackage(watchdogExportDir, watchdogContainerPackageDir)
      const containerWatchdogPort = await freePort()
      const containerWatchdogUrl = `http://127.0.0.1:${containerWatchdogPort}`
      const containerWatchdogName = `arroba-publication-watchdog-${process.pid}`
      dockerContainers.push(containerWatchdogName)
      containerProcess = startPublicationContainer({
        image: publicationContainerImage,
        name: containerWatchdogName,
        packageDir: watchdogContainerPackageDir,
        workspaceDir: watchdogWorkspace,
        port: containerWatchdogPort,
      })
      try {
        await waitForContainerGateway(containerWatchdogUrl, containerProcess, 60_000)
        const containerWatchdogStatusResponse = await fetch(`${containerWatchdogUrl}/.well-known/arroba/publication/status`)
        const containerWatchdogStatusBody = await containerWatchdogStatusResponse.json()
        if (
          containerWatchdogStatusResponse.status !== 200
          || typeof containerWatchdogStatusBody.runtime_session_id !== 'string'
          || containerWatchdogStatusBody.watchdog_count !== 1
          || containerWatchdogStatusBody.watchdogs?.[0]?.id !== watchdog.id
        ) {
          throw new Error(`expected container watchdog status with runtime session and watchdog, got ${containerWatchdogStatusResponse.status}: ${JSON.stringify(containerWatchdogStatusBody)}`)
        }
        const containerWatchdogOutput = await waitForPublicationStatusLatestOutput(
          containerWatchdogUrl,
          watchdogRun.final_output?.message,
        )
        logStep('container_watchdog_ok', {
          runtimeSessionId: containerWatchdogStatusBody.runtime_session_id,
          latestOutput: containerWatchdogOutput.latest_output?.message,
        })
      } finally {
        await stopProcess(containerProcess)
        await removeDockerContainer(containerWatchdogName).catch(() => {})
        containerProcess = null
      }

      logStep('container_missing_requirements_fail_before_listen')
      const missingContainerPackageDir = path.join(root, 'container-missing-requirements')
      await createContainerPortablePackage(apiSseContainerExportDir, missingContainerPackageDir)
      await writeFile(path.join(missingContainerPackageDir, 'requirements.json'), JSON.stringify({
        schema_version: 1,
        skills: [{ name: 'missing-container-publication-skill' }],
        credentials: [{ name: 'missing-container-publication-credential' }],
      }, null, 2))
      const containerMissingPort = await freePort()
      const containerMissingUrl = `http://127.0.0.1:${containerMissingPort}`
      const containerMissingName = `arroba-publication-missing-${process.pid}`
      dockerContainers.push(containerMissingName)
      containerProcess = startPublicationContainer({
        image: publicationContainerImage,
        name: containerMissingName,
        packageDir: missingContainerPackageDir,
        workspaceDir: apiSseWorkspace,
        port: containerMissingPort,
      })
      const failedContainerServe = await waitForProcessExit(containerProcess, 60_000)
      await assertGatewayDoesNotListen(containerMissingUrl)
      if (failedContainerServe.code === 0) {
        throw new Error('expected container missing-requirements serve to fail')
      }
      if (!/publication requirements are missing: skill:missing-container-publication-skill, credential:missing-container-publication-credential/.test(containerProcess.logs.stderr)) {
        throw new Error(`expected container missing-requirements error, got stdout:\n${containerProcess.logs.stdout}\nstderr:\n${containerProcess.logs.stderr}`)
      }
      await removeDockerContainer(containerMissingName).catch(() => {})
      containerProcess = null
      logStep('container_missing_requirements_fail_before_listen_ok')
    } else {
      logStep('container_publication_skipped', { reason: 'set ARROBA_PUBLICATION_CONTAINER_DRILL=1 to run Docker container validation' })
    }

    logStep('missing_requirements_fail_before_listen')
    await writeFile(path.join(exportDir, 'requirements.json'), JSON.stringify({
      schema_version: 1,
      skills: [{ name: 'missing-publication-skill' }],
      credentials: [{ name: 'missing-publication-credential' }],
    }, null, 2))
    gateway = startProcess(
      cliBinary,
      ['serve', exportDir, String(gatewayPort), '--kernel-url', kernelUrl],
      {
        ...env,
        HOST: '127.0.0.1',
      },
      'arroba-serve-missing-requirements',
    )
    const failedServe = await waitForProcessExit(gateway)
    await assertGatewayDoesNotListen(gatewayUrl)
    if (failedServe.code === 0) {
      throw new Error('expected missing-requirements serve to fail')
    }
    if (!/publication requirements are missing: skill:missing-publication-skill, credential:missing-publication-credential/.test(gateway.logs.stderr)) {
      throw new Error(`expected missing-requirements error, got stderr:\n${gateway.logs.stderr}`)
    }
    gateway = null

    logStep('ok', {
      anonymousPublicationId: publication.id,
      workflowRunId: body.workflow_run?.id ?? null,
    })
    succeeded = true
  } finally {
    if (client) {
      for (const id of sessionIds.reverse()) {
        await client.send(endSessionRequest(id)).catch(() => {})
      }
    }
    await client?.close?.().catch(() => {})
    await stopProcess(gateway)
    await stopProcess(kernel)
    await stopProcess(relay)
    for (const name of dockerContainers.reverse()) {
      await removeDockerContainer(name).catch(() => {})
    }
    for (const image of dockerImages.reverse()) {
      await removeDockerImage(image).catch(() => {})
    }
    if (!succeeded) {
      console.error('[publication-drill] relay logs', relay?.logs ?? null)
      console.error('[publication-drill] kernel logs', kernel?.logs ?? null)
      console.error('[publication-drill] gateway logs', gateway?.logs ?? null)
    }
    await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 })
  }
}

main().catch((error) => {
  console.error(`[publication-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
