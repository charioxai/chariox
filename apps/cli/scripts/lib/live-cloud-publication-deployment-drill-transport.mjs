import { spawn } from 'node:child_process'
import net from 'node:net'
import path from 'node:path'
import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { WebSocket } from 'ws'
import { LocalIpcClient } from '../../../../packages/kernel-client/dist/ipc.js'
import {
  getDaemonHealthRequest,
  getProviderRunRequest,
} from '../../../../packages/kernel-client/dist/ipc-requests.js'
import {
  getPublicationDeployment,
  listPublicationDeploymentLogs,
} from '../../dist/publication-deployment-api.js'
import {
  rustBinaryPath,
  rustManifestPath,
} from '../../../../scripts/rust-workspace.mjs'
import { publicationStatusWatchdogCount, publicationStatusWatchdogs } from './live-workflow-publication-drill-runtime.mjs'

const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..', '..', '..', '..')

const SHOPPING_LIST_PROMPT_B = '2 red apples, 1 bag of coffee beans, and 3 packs of pasta'
const SHOPPING_EXPECTED_SNIPPETS = ['Agent App Grocery Checkout', 'data-arroba-agent-app-checkout', 'bananas', 'Coca-Cola', 'chips']
const SHOPPING_EXPECTED_SNIPPETS_B = ['Agent App Grocery Checkout', 'data-arroba-agent-app-checkout', 'apples', 'coffee', 'pasta']

export async function validateTransport(input) {
  const base = input.publicBaseUrl.replace(/\/+$/, '')
  if (input.transport === 'human_http') {
    const promptUrl = input.expectAgentAppShopping
      ? `${base}/add/${encodeURIComponent(input.prompt)}`
      : `${base}/final/${encodeURIComponent(input.prompt)}`
    if (input.browserScreenshot) {
      if (input.expectAgentAppShopping) {
        if (input.agentAppSessionIsolation) {
          return await runAgentAppShoppingSessionIsolation({
            baseUrl: base,
            prompt: input.prompt,
            secondPrompt: SHOPPING_LIST_PROMPT_B,
            artifactsDir: input.artifactsDir,
            slug: input.slug,
          })
        }
        return await runAgentAppShoppingBrowserScreenshot({
          url: promptUrl,
          artifactsDir: input.artifactsDir,
          slug: input.slug,
          expectedSnippets: SHOPPING_EXPECTED_SNIPPETS,
        })
      }
      return await runHumanHttpDashboardBrowserScreenshot({
        url: promptUrl,
        artifactsDir: input.artifactsDir,
        slug: input.slug,
      })
    }
    const response = await fetch(promptUrl, { headers: { accept: 'text/html' } })
    const body = await response.text()
    if (!response.ok || !body.includes('EventSource')) throw new Error(`human HTTP viewer failed: ${response.status} ${body.slice(0, 200)}`)
    const viewerConfig = parseHumanHttpViewerConfig(body)
    if (!viewerConfig.eventsUrl) throw new Error(`human HTTP viewer did not expose an events URL:\n${body.slice(0, 1000)}`)
    const eventTranscript = await readSse(`${base}${viewerConfig.eventsUrl}`, null, { method: 'GET' })
    if (!eventTranscript.includes('event: final')) throw new Error(`human HTTP event transcript missing final:\n${eventTranscript}`)
    if (!eventTranscript.includes('event: trace')) throw new Error(`human HTTP event transcript missing trace:\n${eventTranscript}`)
    assertSuccessfulSseTranscript(eventTranscript, 'human HTTP')
    if (input.expectHtmlDashboard) {
      for (const snippet of ['Real Provider Workflow Dashboard', 'data-arroba-real-provider-dashboard']) {
        if (!eventTranscript.includes(snippet)) {
          throw new Error(`human HTTP final transcript missing dashboard snippet ${snippet}:\n${eventTranscript}`)
        }
      }
    }
    const htmlPath = path.join(input.artifactsDir, `${input.slug}-human-http-viewer.html`)
    const transcriptPath = path.join(input.artifactsDir, `${input.slug}-human-http-events.txt`)
    await writeFile(htmlPath, body)
    await writeFile(transcriptPath, eventTranscript)
    return { promptUrl, htmlPath, transcriptPath }
  }
  if (input.transport === 'api_sse_json') {
    const body = await readSse(`${base}/invoke`, { prompt: input.prompt })
    const transcriptPath = path.join(input.artifactsDir, `${input.slug}-api-sse.txt`)
    await writeFile(transcriptPath, body)
    for (const event of ['queued', 'started', 'trace', 'final']) {
      if (!body.includes(`event: ${event}`)) throw new Error(`API SSE transcript missing ${event}:\n${body}`)
    }
    assertSuccessfulSseTranscript(body, 'API SSE')
    if (input.expectHtmlDashboard) {
      for (const snippet of ['Real Provider Workflow Dashboard', 'data-arroba-real-provider-dashboard']) {
        if (!body.includes(snippet)) throw new Error(`API SSE final transcript missing dashboard snippet ${snippet}:\n${body}`)
      }
    }
    return { transcriptPath }
  }
  if (input.transport === 'websocket_json') {
    const events = await invokeWebSocket(`${base}/.well-known/arroba/publication/ws`, { prompt: input.prompt })
    const transcriptPath = path.join(input.artifactsDir, `${input.slug}-websocket.json`)
    await writeFile(transcriptPath, `${JSON.stringify(events, null, 2)}\n`)
    for (const type of ['ready', 'accepted', 'trace', 'final']) {
      if (!events.some((event) => event.type === type)) throw new Error(`WebSocket transcript missing ${type}: ${JSON.stringify(events)}`)
    }
    assertSuccessfulWebSocketEvents(events)
    if (!events.some((event) => event.type === 'queued' || event.type === 'started' || event.type === 'status')) {
      throw new Error(`WebSocket transcript missing queued/started/status progress event: ${JSON.stringify(events)}`)
    }
    if (input.expectHtmlDashboard) {
      const body = JSON.stringify(events)
      for (const snippet of ['Real Provider Workflow Dashboard', 'data-arroba-real-provider-dashboard']) {
        if (!body.includes(snippet)) throw new Error(`WebSocket final transcript missing dashboard snippet ${snippet}: ${body}`)
      }
    }
    return { transcriptPath }
  }
  if (input.transport === 'mcp') {
    const listResponse = await fetch(`${base}/mcp`, {
      method: 'POST',
      headers: { accept: 'application/json', 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list', params: {} }),
    })
    const transcriptPath = path.join(input.artifactsDir, `${input.slug}-mcp-tools-list.json`)
    const listBody = await listResponse.text()
    await writeFile(transcriptPath, listBody)
    if (!listResponse.ok || !listBody.includes('tools')) throw new Error(`MCP tools/list failed: ${listResponse.status} ${listBody}`)
    const toolName = JSON.parse(listBody)?.result?.tools?.[0]?.name
    if (!toolName) throw new Error(`MCP tools/list did not return a tool name: ${listBody}`)
    const callResponse = await fetch(`${base}/mcp`, {
      method: 'POST',
      headers: { accept: 'application/json', 'content-type': 'application/json' },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: 2,
        method: 'tools/call',
        params: { name: toolName, arguments: { prompt: input.prompt } },
      }),
    })
    const callBody = await callResponse.text()
    const callTranscriptPath = path.join(input.artifactsDir, `${input.slug}-mcp-tools-call.json`)
    await writeFile(callTranscriptPath, callBody)
    if (!callResponse.ok || !callBody.includes('content')) throw new Error(`MCP tools/call failed: ${callResponse.status} ${callBody}`)
    assertSuccessfulMcpToolCall(callBody)
    if (input.expectHtmlDashboard) {
      for (const snippet of ['Real Provider Workflow Dashboard', 'data-arroba-real-provider-dashboard']) {
        if (!callBody.includes(snippet)) throw new Error(`MCP tools/call final missing dashboard snippet ${snippet}:\n${callBody}`)
      }
    }
    return { transcriptPath, callTranscriptPath }
  }
  if (input.transport === 'schedule') {
    const status = await waitForSchedulePublicationStatus(base, {
      expectHtmlDashboard: input.expectHtmlDashboard,
    })
    const statusPath = path.join(input.artifactsDir, `${input.slug}-schedule-status.json`)
    await writeFile(statusPath, `${JSON.stringify(status, null, 2)}\n`)
    return { statusPath, latestOutput: status.latest_output?.message ?? null }
  }
  throw new Error(`unsupported transport ${input.transport}`)
}

export async function waitForSchedulePublicationStatus(base, options = {}) {
  const statusUrl = `${base}/.well-known/arroba/publication/status`
  const deadline = Date.now() + 900_000
  let last = null
  while (Date.now() < deadline) {
    const response = await fetch(statusUrl, { headers: { accept: 'application/json' } })
    const body = await response.text()
    if (!response.ok) throw new Error(`schedule status failed: ${response.status} ${body}`)
    last = JSON.parse(body)
    const watchdogs = publicationStatusWatchdogs(last)
    if (publicationStatusWatchdogCount(last) !== 1 || watchdogs.length !== 1) {
      throw new Error(`schedule status did not expose exactly one schedule: ${body}`)
    }
    const latest = last.latest_output?.message
    const schedule = watchdogs[0]
    const status = String(schedule.last_status ?? '').toLowerCase()
    if (latest && ['started', 'completed_budget'].includes(status)) {
      if (options.expectHtmlDashboard) {
        const serialized = JSON.stringify(last)
        for (const snippet of ['Real Provider Workflow Dashboard', 'data-arroba-real-provider-dashboard']) {
          if (!serialized.includes(snippet)) throw new Error(`schedule latest output missing dashboard snippet ${snippet}:\n${serialized}`)
        }
      }
      return last
    }
    if (schedule.last_error) {
      throw new Error(`schedule failed: ${schedule.last_error}\n${body}`)
    }
    await delay(2_000)
  }
  throw new Error(`schedule publication did not produce latest output: ${JSON.stringify(last, null, 2)}`)
}

export function assertSuccessfulSseTranscript(transcript, label) {
  const frames = parseSseTranscript(transcript)
  const finalFrame = [...frames].reverse().find((frame) => frame.event === 'final')
  const workflowRun = finalFrame?.data?.workflow_run ?? null
  if (!workflowRun) throw new Error(`${label} transcript final event did not include workflow_run:\n${transcript}`)
  assertWorkflowRunCompleted(workflowRun, `${label} transcript`)
}

export function parseSseTranscript(transcript) {
  const frames = []
  for (const frame of transcript.split(/\r?\n\r?\n/)) {
    if (!frame.trim()) continue
    let event = 'message'
    const data = []
    for (const line of frame.split(/\r?\n/)) {
      if (line.startsWith('event:')) event = line.slice(6).trim()
      if (line.startsWith('data:')) data.push(line.slice(5).trimStart())
    }
    if (!data.length) continue
    try {
      frames.push({ event, data: JSON.parse(data.join('\n')) })
    } catch (error) {
      throw new Error(`could not parse ${event} SSE frame as JSON: ${errorMessage(error)}\n${frame}`)
    }
  }
  return frames
}

export function assertSuccessfulWebSocketEvents(events) {
  const finalEvent = [...events].reverse().find((event) => event.type === 'final')
  if (!finalEvent?.workflow_run) throw new Error(`WebSocket final event did not include workflow_run: ${JSON.stringify(events)}`)
  assertWorkflowRunCompleted(finalEvent.workflow_run, 'WebSocket transcript')
}

export function assertSuccessfulMcpToolCall(callBody) {
  let payload
  try {
    payload = JSON.parse(callBody)
  } catch (error) {
    throw new Error(`MCP tools/call response was not JSON: ${errorMessage(error)}\n${callBody}`)
  }
  const result = payload?.result
  const structured = result?.structuredContent
  if (!structured) throw new Error(`MCP tools/call response missing structuredContent:\n${callBody}`)
  if (structured.status !== 'Completed' || result.isError) {
    throw new Error(`MCP tools/call workflow did not complete successfully:\n${callBody}`)
  }
  if (JSON.stringify(structured).includes('provider_failure')) {
    throw new Error(`MCP tools/call exposed provider failure:\n${callBody}`)
  }
}

export function assertWorkflowRunCompleted(workflowRun, label) {
  if (workflowRun.status !== 'Completed') {
    throw new Error(`${label} workflow status was ${workflowRun.status}, expected Completed:\n${JSON.stringify(workflowRun, null, 2)}`)
  }
  const failures = workflowRun.failure_events ?? []
  if (failures.length > 0) {
    throw new Error(`${label} workflow had failure events:\n${JSON.stringify(failures, null, 2)}`)
  }
}

export function parseHumanHttpViewerConfig(body) {
  const match = body.match(/window\.__arrobaPublicationViewerConfig\s*=\s*(\{.*?\});/s)
  if (!match) return {}
  return JSON.parse(match[1])
}

export async function readSse(url, payload, options = {}) {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(), options.timeoutMs ?? 300_000)
  const response = await fetch(url, {
    method: options.method ?? 'POST',
    headers: payload == null
      ? { accept: 'text/event-stream' }
      : { accept: 'text/event-stream', 'content-type': 'application/json' },
    ...(payload == null ? {} : { body: JSON.stringify(payload) }),
    signal: controller.signal,
  }).finally(() => clearTimeout(timeout))
  const body = await response.text()
  if (!response.ok) throw new Error(`SSE failed: ${response.status} ${body}`)
  return body
}

export async function invokeWebSocket(url, payload) {
  const socket = new WebSocket(url)
  const events = []
  return await new Promise((resolve, reject) => {
    let invoked = false
    const timeout = setTimeout(() => {
      socket.close()
      reject(new Error(`timed out waiting for websocket event; events=${JSON.stringify(events)}`))
    }, 300_000)
    socket.on('message', (data) => {
      try {
        const event = JSON.parse(data.toString())
        events.push(event)
        if (event.type === 'ready' && !invoked) {
          invoked = true
          socket.send(JSON.stringify({ type: 'invoke', input: payload }))
        }
        if (event.type === 'final') {
          clearTimeout(timeout)
          socket.close()
          resolve(events)
        }
        if (event.type === 'error') {
          clearTimeout(timeout)
          socket.close()
          reject(new Error(`websocket error: ${event.error ?? 'unknown'}; events=${JSON.stringify(events)}`))
        }
      } catch (error) {
        clearTimeout(timeout)
        socket.close()
        reject(error)
      }
    })
    socket.on('error', (error) => {
      clearTimeout(timeout)
      reject(error)
    })
    socket.on('close', () => {
      if (!events.some((event) => event.type === 'final')) {
        clearTimeout(timeout)
        reject(new Error(`websocket closed before final; events=${JSON.stringify(events)}`))
      }
    })
  })
}

export async function runHumanHttpDashboardBrowserScreenshot({ url, artifactsDir, slug }) {
  const chromePath = await findChromeExecutable()
  if (!chromePath) throw new Error('Chrome executable was not found for browser screenshot validation')
  const debuggingPort = await freePort()
  const userDataDir = path.join(artifactsDir, `${slug}-chrome-profile`)
  const screenshotPath = path.join(artifactsDir, `${slug}-browser-final-dashboard.png`)
  await rm(userDataDir, { recursive: true, force: true })
  await mkdir(userDataDir, { recursive: true })
  const chrome = startProcess(
    chromePath,
    [
      '--headless=new',
      '--disable-gpu',
      '--disable-background-networking',
      '--disable-extensions',
      '--window-size=1440,1000',
      '--no-first-run',
      '--no-default-browser-check',
      '--no-sandbox',
      '--remote-debugging-address=127.0.0.1',
      `--remote-debugging-port=${debuggingPort}`,
      `--user-data-dir=${userDataDir}`,
      'about:blank',
    ],
    process.env,
    'chrome-cloud-publication-dashboard',
  )
  let cdp = null
  try {
    const target = await waitForChromeTarget(debuggingPort, chrome)
    cdp = await connectChromeTarget(target.webSocketDebuggerUrl)
    await cdp.send('Page.enable')
    await cdp.send('Runtime.enable')
    await withTimeout(cdp.send('Page.navigate', { url }), 10_000, `browser navigate ${url}`)
    const finalState = await waitForBrowserDashboardFinal(cdp, 420_000)
    const screenshot = await withTimeout(
      cdp.send('Page.captureScreenshot', { format: 'png', fromSurface: true }),
      10_000,
      'browser dashboard screenshot',
    )
    if (typeof screenshot.data !== 'string' || screenshot.data.length < 1000) {
      throw new Error('browser dashboard screenshot was empty')
    }
    await writeFile(screenshotPath, Buffer.from(screenshot.data, 'base64'))
    return { promptUrl: url, screenshotPath, finalState }
  } catch (error) {
    throw new Error(`${errorMessage(error)}\nchrome stdout:\n${chrome.logs.stdout}\nchrome stderr:\n${chrome.logs.stderr}`)
  } finally {
    await cdp?.close?.().catch(() => {})
    await stopProcess(chrome)
    await rm(userDataDir, { recursive: true, force: true }).catch(() => {})
  }
}

export async function runAgentAppShoppingSessionIsolation({ baseUrl, prompt, secondPrompt, artifactsDir, slug }) {
  const firstUrl = `${baseUrl}/add/${encodeURIComponent(prompt)}`
  const secondUrl = `${baseUrl}/add/${encodeURIComponent(secondPrompt)}`
  const first = await runAgentAppShoppingBrowserScreenshot({
    url: firstUrl,
    artifactsDir,
    slug,
    expectedSnippets: SHOPPING_EXPECTED_SNIPPETS,
    screenshotName: 'agent-app-shopping-checkout-session-a',
  })
  const second = await runAgentAppShoppingBrowserScreenshot({
    url: secondUrl,
    artifactsDir,
    slug,
    expectedSnippets: SHOPPING_EXPECTED_SNIPPETS_B,
    screenshotName: 'agent-app-shopping-checkout-session-b',
  })
  if (!first.cookieHeader) throw new Error('first Agent App session did not expose a browser session cookie')
  if (!first.finalState?.iframeSrc) throw new Error('first Agent App session did not expose a generated checkout iframe URL')
  const firstCheckoutUrl = new URL(first.finalState.iframeSrc, firstUrl)
  const revisited = await fetch(firstCheckoutUrl, {
    headers: { cookie: first.cookieHeader },
  })
  const revisitedHtml = await revisited.text()
  const lowerRevisitedHtml = revisitedHtml.toLowerCase()
  const firstMissing = SHOPPING_EXPECTED_SNIPPETS.filter((snippet) => !lowerRevisitedHtml.includes(snippet.toLowerCase()))
  const leakedSecond = SHOPPING_EXPECTED_SNIPPETS_B.filter((snippet) => (
    !SHOPPING_EXPECTED_SNIPPETS.some((firstSnippet) => firstSnippet.toLowerCase() === snippet.toLowerCase())
    && lowerRevisitedHtml.includes(snippet.toLowerCase())
  ))
  if (!revisited.ok || firstMissing.length || leakedSecond.length) {
    throw new Error(`Agent App session isolation failed: ${JSON.stringify({
      status: revisited.status,
      firstMissing,
      leakedSecond,
      firstCheckoutUrl: firstCheckoutUrl.toString(),
    })}`)
  }
  return {
    promptUrl: firstUrl,
    secondPromptUrl: secondUrl,
    screenshotPath: first.screenshotPath,
    secondScreenshotPath: second.screenshotPath,
    finalState: first.finalState,
    secondFinalState: second.finalState,
    isolation: { firstCheckoutUrl: firstCheckoutUrl.toString(), status: revisited.status },
  }
}

export async function runAgentAppShoppingBrowserScreenshot({
  url,
  artifactsDir,
  slug,
  expectedSnippets = SHOPPING_EXPECTED_SNIPPETS,
  screenshotName = 'agent-app-shopping-checkout',
}) {
  const chromePath = await findChromeExecutable()
  if (!chromePath) throw new Error('Chrome executable was not found for Agent App browser screenshot validation')
  const debuggingPort = await freePort()
  const userDataDir = path.join(artifactsDir, `${slug}-agent-app-chrome-profile`)
  const screenshotPath = path.join(artifactsDir, `${slug}-${screenshotName}.png`)
  await rm(userDataDir, { recursive: true, force: true })
  await mkdir(userDataDir, { recursive: true })
  const chrome = startProcess(
    chromePath,
    [
      '--headless=new',
      '--disable-gpu',
      '--disable-background-networking',
      '--disable-extensions',
      '--window-size=1440,1000',
      '--no-first-run',
      '--no-default-browser-check',
      '--no-sandbox',
      '--remote-debugging-address=127.0.0.1',
      `--remote-debugging-port=${debuggingPort}`,
      `--user-data-dir=${userDataDir}`,
      'about:blank',
    ],
    process.env,
    'chrome-cloud-agent-app-shopping',
  )
  let cdp = null
  try {
    const target = await waitForChromeTarget(debuggingPort, chrome)
    cdp = await connectChromeTarget(target.webSocketDebuggerUrl)
    await cdp.send('Page.enable')
    await cdp.send('Runtime.enable')
    await cdp.send('Network.enable')
    await withTimeout(cdp.send('Page.navigate', { url }), 10_000, `browser navigate ${url}`)
    const finalState = await waitForAgentAppShoppingFinal(cdp, 600_000, expectedSnippets)
    const screenshot = await withTimeout(
      cdp.send('Page.captureScreenshot', { format: 'png', fromSurface: true }),
      10_000,
      'Agent App shopping screenshot',
    )
    if (typeof screenshot.data !== 'string' || screenshot.data.length < 1000) {
      throw new Error('Agent App shopping screenshot was empty')
    }
    await writeFile(screenshotPath, Buffer.from(screenshot.data, 'base64'))
    const cookieHeader = await agentAppCookieHeader(cdp)
    return { promptUrl: url, screenshotPath, finalState, cookieHeader }
  } catch (error) {
    throw new Error(`${errorMessage(error)}\nchrome stdout:\n${chrome.logs.stdout}\nchrome stderr:\n${chrome.logs.stderr}`)
  } finally {
    await cdp?.close?.().catch(() => {})
    await stopProcess(chrome)
    await rm(userDataDir, { recursive: true, force: true }).catch(() => {})
  }
}

export async function waitForAgentAppShoppingFinal(cdp, timeoutMs, expectedSnippets) {
  const deadline = Date.now() + timeoutMs
  let lastState = null
  const requiredSnippetsJson = JSON.stringify(expectedSnippets)
  while (Date.now() < deadline) {
    const evaluated = await withTimeout(cdp.send('Runtime.evaluate', {
      returnByValue: true,
      awaitPromise: true,
      expression: `(async () => {
        const required = ${requiredSnippetsJson};
        const status = document.querySelector('#status')?.textContent?.trim() || '';
        const iframe = document.querySelector('#html-output iframe');
        const iframeSrcdoc = iframe?.getAttribute('srcdoc') || '';
        const iframeSrc = iframe?.getAttribute('src') || '';
        let fetchedHtml = '';
        let fetchStatus = null;
        if (!iframeSrcdoc && iframeSrc) {
          try {
            const response = await fetch(iframeSrc, { cache: 'no-store' });
            fetchStatus = response.status;
            fetchedHtml = await response.text();
          } catch (error) {
            fetchedHtml = String(error?.message || error);
          }
        }
        const iframeDocumentHtml = iframe?.contentDocument?.documentElement?.outerHTML || '';
        const renderedHtml = iframeSrcdoc || fetchedHtml || iframeDocumentHtml;
        const traceText = Array.from(document.querySelectorAll('#trace-feed .trace-item')).map((item) => item.textContent || '').join('\\n');
        const lowerRenderedHtml = renderedHtml.toLowerCase();
        const missing = required.filter((snippet) => !lowerRenderedHtml.includes(String(snippet).toLowerCase()));
        return {
          status,
          missing,
          iframeSrc,
          fetchStatus,
          renderedLength: renderedHtml.length,
          traceText,
          traceCount: document.querySelectorAll('#trace-feed .trace-item').length,
          actionTraceOk: traceText.includes('agent_app_action') || traceText.includes('arroba.agent_app_action') || traceText.includes('cart.add'),
          ok: status === 'Completed' && missing.length === 0 && (traceText.includes('cart.add') || traceText.includes('agent_app_action')),
        };
      })()`,
    }), 15_000, 'Agent App shopping Runtime.evaluate')
    lastState = evaluated.result?.value ?? null
    if (lastState?.ok) return lastState
    if (
      lastState?.status === 'Completed'
      && Number(lastState?.renderedLength ?? 0) > 0
      && Array.isArray(lastState?.missing)
      && lastState.missing.length > 0
    ) {
      throw new Error(`Agent App shopping completed without checkout snippets: ${JSON.stringify(lastState)}`)
    }
    await delay(750)
  }
  throw new Error(`Agent App shopping browser did not render final checkout: ${JSON.stringify(lastState)}`)
}

export async function agentAppCookieHeader(cdp) {
  const response = await cdp.send('Network.getAllCookies')
  const cookies = Array.isArray(response?.cookies) ? response.cookies : []
  const sessionCookie = cookies.find((cookie) => cookie?.name === 'arroba_agent_app_session')
  if (!sessionCookie?.value) return null
  return `${sessionCookie.name}=${encodeURIComponent(sessionCookie.value)}`
}

export async function findChromeExecutable() {
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

export async function waitForChromeTarget(debuggingPort, chrome) {
  const endpoint = `http://127.0.0.1:${debuggingPort}/json/list`
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const response = await fetch(endpoint)
      const targets = await response.json()
      const target = targets.find((candidate) => candidate.type === 'page' && candidate.webSocketDebuggerUrl)
      if (target?.webSocketDebuggerUrl) return target
    } catch (error) {
      lastError = error
    }
    await delay(250)
  }
  throw new Error(`Chrome DevTools target did not become ready: ${lastError?.message ?? 'no page target'}\n${chrome.logs.stderr.slice(-2_000)}`)
}

export async function connectChromeTarget(webSocketUrl) {
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

export async function waitForBrowserDashboardFinal(cdp, timeoutMs) {
  const requiredTraceLevels = ['output_summary', 'assistant_messages', 'thinking', 'tool_use']
  const requiredHtmlSnippets = ['Real Provider Workflow Dashboard', 'data-arroba-real-provider-dashboard']
  const deadline = Date.now() + timeoutMs
  let lastState = null
  while (Date.now() < deadline) {
    const evaluated = await withTimeout(cdp.send('Runtime.evaluate', {
      returnByValue: true,
      expression: `(() => {
        const status = document.querySelector('#status')?.textContent?.trim() || '';
        const iframe = document.querySelector('#html-output iframe');
        const iframeSrcdoc = iframe?.getAttribute('srcdoc') || '';
        const traces = Array.from(document.querySelectorAll('#trace-feed .trace-item')).map((item) => ({
          text: item.textContent || '',
          meta: Array.from(item.querySelectorAll('.trace-meta span')).map((span) => (span.textContent || '').trim()),
        }));
        const requiredTraceLevels = ${JSON.stringify(requiredTraceLevels)};
        const traceLevels = Array.from(new Set(traces.flatMap((trace) => trace.meta).filter((value) => requiredTraceLevels.includes(value)))).sort();
        const traceAliases = Array.from(new Set(traces.map((trace) => trace.meta[0]).filter(Boolean))).sort();
        const missingTraceLevels = requiredTraceLevels.filter((level) => !traceLevels.includes(level));
        const htmlOk = ${JSON.stringify(requiredHtmlSnippets)}.every((snippet) => iframeSrcdoc.includes(snippet));
        return {
          status,
          htmlOk,
          traceCount: traces.length,
          traceLevels,
          traceAliases,
          missingTraceLevels,
          ok: status === 'Completed' && htmlOk && traces.length > 0 && missingTraceLevels.length === 0,
        };
      })()`,
    }), 15_000, 'browser dashboard Runtime.evaluate')
    lastState = evaluated.result?.value ?? null
    if (lastState?.ok) return lastState
    if (
      lastState?.status === 'Completed'
      && lastState?.htmlOk
      && Array.isArray(lastState?.missingTraceLevels)
      && lastState.missingTraceLevels.length > 0
    ) {
      throw new Error(`browser completed without required trace levels: ${JSON.stringify(lastState)}`)
    }
    await delay(500)
  }
  throw new Error(`browser did not render final dashboard: ${JSON.stringify(lastState)}`)
}

export async function withTimeout(promise, timeoutMs, label) {
  let timer = null
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timed out after ${timeoutMs}ms`)), timeoutMs)
      }),
    ])
  } finally {
    if (timer) clearTimeout(timer)
  }
}

export async function waitForDeploymentReady(profile, deploymentId) {
  const deadline = Date.now() + 420_000
  let last = null
  while (Date.now() < deadline) {
    last = await getPublicationDeployment(profile, deploymentId)
    if (last.status === 'ready') return last
    if (last.status === 'failed' || last.status === 'unavailable') {
      throw new Error(`deployment ${last.status}: ${last.lastError ?? 'unknown error'}`)
    }
    await delay(2_000)
  }
  throw new Error(`deployment did not become ready: ${JSON.stringify(last)}`)
}

export async function waitForDeploymentActionAuditLogs(profile, deploymentId) {
  const deadline = Date.now() + 60_000
  let lastLogs = []
  while (Date.now() < deadline) {
    lastLogs = await listPublicationDeploymentLogs(profile, deploymentId)
    const text = lastLogs.map((entry) => entry.message).join('\n')
    if (text.includes('agent app action `cart.add` completed') && text.includes('agent app action `cart.checkout` completed')) {
      return lastLogs
    }
    await delay(1_000)
  }
  throw new Error(`deployment logs did not include Agent App action audit entries: ${JSON.stringify(lastLogs.slice(-20))}`)
}

export async function waitForProviderRunReady(client, providerRunId) {
  const deadline = Date.now() + 120_000
  let last = null
  while (Date.now() < deadline) {
    const response = await client.send(getProviderRunRequest(providerRunId)).catch(() => null)
    last = response?.ProviderRun?.provider_run ?? response?.provider_run ?? response
    const state = String(last?.status ?? last?.state ?? '').toLowerCase()
    if (state === 'running' || state === 'ready') return
    if (state === 'failed' || state === 'exited' || state === 'stopped') {
      throw new Error(`provider run ${providerRunId} ended before ready: ${JSON.stringify(last)}`)
    }
    await delay(500)
  }
  throw new Error(`provider run ${providerRunId} did not become ready: ${JSON.stringify(last)}`)
}

export async function buildRustBinary(binaryName) {
  const binaryPath = rustBinaryPath(repoRoot, binaryName)
  const exists = await readFile(binaryPath).then(() => true).catch(() => false)
  if (exists) return binaryPath
  const manifestPath = rustManifestPath(repoRoot, binaryName)
  const result = await run('cargo', ['build', '--manifest-path', manifestPath, '--bin', binaryName])
  if (result.code !== 0) throw new Error(`cargo build ${binaryName} failed\n${result.stdout}\n${result.stderr}`)
  return binaryPath
}

export function startProcess(command, args, env, name) {
  const logs = { stdout: '', stderr: '' }
  const child = spawn(command, args, { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })
  child.stdout.on('data', (chunk) => { logs.stdout += chunk.toString() })
  child.stderr.on('data', (chunk) => { logs.stderr += chunk.toString() })
  child.logs = logs
  child.name = name
  return child
}

export async function stopProcess(child) {
  if (!child || child.killed) return
  if (child.exitCode !== null || child.signalCode !== null) return
  child.kill('SIGTERM')
  const result = await waitForProcessExit(child, 5_000).catch(async () => {
    child.kill('SIGKILL')
    return waitForProcessExit(child, 5_000)
  })
  return result
}

export function waitForProcessExit(child, timeoutMs) {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error(`timed out waiting for ${child.name ?? child.pid}`)), timeoutMs)
    child.once('exit', (code, signal) => {
      clearTimeout(timeout)
      resolve({ code, signal })
    })
  })
}

export async function waitForKernel(kernelUrl) {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(getDaemonHealthRequest())
      await client.close?.()
      return
    } catch {
      await client.close?.().catch(() => {})
      await delay(250)
    }
  }
  throw new Error(`kernel did not become ready at ${kernelUrl}`)
}

export async function waitForGateway(baseUrl) {
  const deadline = Date.now() + 60_000
  const statusUrl = `${baseUrl.replace(/\/+$/, '')}/.well-known/arroba/publication/status`
  while (Date.now() < deadline) {
    try {
      const response = await fetch(statusUrl)
      if (response.ok) return
    } catch {}
    await delay(250)
  }
  throw new Error(`gateway did not become ready at ${statusUrl}`)
}

export async function waitForTcpPort(host, port) {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    if (await canConnect(host, port)) return
    await delay(250)
  }
  throw new Error(`timed out waiting for ${host}:${port}`)
}

export function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer()
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      const port = typeof address === 'object' && address ? address.port : null
      server.close(() => port ? resolve(port) : reject(new Error('could not allocate free port')))
    })
    server.once('error', reject)
  })
}

export function canConnect(host, port) {
  return new Promise((resolve) => {
    const socket = net.connect({ host, port })
    socket.once('connect', () => {
      socket.end()
      resolve(true)
    })
    socket.once('error', () => resolve(false))
  })
}

export function run(command, args, options = {}) {
  return new Promise((resolve) => {
    const child = spawn(command, args, { cwd: options.cwd ?? repoRoot, env: options.env ?? process.env, stdio: ['ignore', 'pipe', 'pipe'] })
    const stdout = []
    const stderr = []
    child.stdout.on('data', (chunk) => stdout.push(Buffer.from(chunk)))
    child.stderr.on('data', (chunk) => stderr.push(Buffer.from(chunk)))
    child.on('close', (code) => resolve({
      code,
      stdout: Buffer.concat(stdout).toString('utf8'),
      stderr: Buffer.concat(stderr).toString('utf8'),
    }))
  })
}

export async function copyCloudProfile(configHome) {
  const source = path.join(process.env.HOME, '.arroba', 'config.json')
  const target = path.join(configHome, 'arroba', 'config.json')
  await cp(source, target)
}

export function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

export function errorMessage(error) {
  return error instanceof Error ? error.message : String(error)
}
