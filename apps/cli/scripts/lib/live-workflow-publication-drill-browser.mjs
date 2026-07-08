import path from 'node:path'
import { cp, mkdir, writeFile } from 'node:fs/promises'
import { WebSocket } from 'ws'
import { freePort, logStep, run, startProcess, stopProcess, withTimeout } from './live-workflow-publication-drill-runtime.mjs'

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

export async function runHumanHttpBrowserDrill({ url, root, timeoutMs = 30_000 }) {
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
    'chrome-human-http-publication',
  )
  let cdp = null
  try {
    const target = await waitForChromeTarget(debuggingPort, 'about:blank', chrome)
    cdp = await connectChromeTarget(target.webSocketDebuggerUrl)
    await cdp.send('Page.enable')
    await cdp.send('Runtime.enable')
    await cdp.send('Page.addScriptToEvaluateOnNewDocument', { source: browserStatusRecorderScript() })
    await withTimeout(cdp.send('Page.navigate', { url }), 10_000, `browser navigate ${url}`)
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
    await preserveVisualArtifact(screenshotPath, 'local-human-http-final.png')
    logStep('browser_screenshot_ok', { screenshotPath, statuses, outputs })
  } finally {
    await cdp?.close?.().catch(() => {})
    await stopProcess(chrome)
  }
}

export async function runHumanHttpHtmlFinalBrowserDrill({
  url,
  root,
  timeoutMs = 30_000,
  expectedHtmlText = 'Vibrant Workflow Dashboard',
  requiredHtmlSnippets = [],
  requiredTraceLevels = [],
  requiredTraceAlias = null,
  visualArtifactPrefix = 'local-human-http-dashboard',
  requirePartial = true,
}) {
  const chromePath = await findChromeExecutable()
  if (!chromePath) {
    logStep('browser_html_final_screenshot_skipped', { reason: 'chrome-not-found' })
    return
  }
  const debuggingPort = await freePort()
  const userDataDir = path.join(root, 'chrome-html-final-profile')
  const partialScreenshotPath = path.join(root, 'human-http-html-partial.png')
  const screenshotPath = path.join(root, 'human-http-html-final.png')
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
    'chrome-human-http-html-final-publication',
  )
  let cdp = null
  try {
    const target = await waitForChromeTarget(debuggingPort, 'about:blank', chrome)
    cdp = await connectChromeTarget(target.webSocketDebuggerUrl)
    await cdp.send('Page.enable')
    await cdp.send('Runtime.enable')
    await cdp.send('Page.addScriptToEvaluateOnNewDocument', { source: browserStatusRecorderScript() })
    await cdp.send('Page.navigate', { url })
    let partialState = null
    if (requirePartial) {
      partialState = await waitForBrowserHtmlPartialOutput(cdp, timeoutMs)
      const partialScreenshot = await captureBrowserScreenshot(cdp, 'browser HTML partial screenshot')
      if (typeof partialScreenshot.data !== 'string' || partialScreenshot.data.length < 1000) {
        throw new Error('browser HTML partial screenshot was empty')
      }
      await writeFile(partialScreenshotPath, Buffer.from(partialScreenshot.data, 'base64'))
      await preserveVisualArtifact(partialScreenshotPath, `${visualArtifactPrefix}-partial.png`)
    }
    let finalState = null
    try {
      finalState = await waitForBrowserHtmlFinalOutput(cdp, {
      timeoutMs,
      expectedHtmlText,
      requiredHtmlSnippets,
      requiredTraceLevels,
      requiredTraceAlias,
    })
    } catch (error) {
      const failedScreenshot = await captureBrowserScreenshot(cdp, 'browser HTML failed screenshot').catch(() => null)
      if (typeof failedScreenshot?.data === 'string' && failedScreenshot.data.length > 1000) {
        const failedScreenshotPath = path.join(root, `${visualArtifactPrefix}-failed.png`)
        await writeFile(failedScreenshotPath, Buffer.from(failedScreenshot.data, 'base64'))
        await preserveVisualArtifact(failedScreenshotPath, `${visualArtifactPrefix}-failed.png`)
      }
      throw error
    }
    const screenshot = await captureBrowserScreenshot(cdp, 'browser HTML final screenshot')
    if (typeof screenshot.data !== 'string' || screenshot.data.length < 1000) {
      throw new Error('browser HTML final screenshot was empty')
    }
    await writeFile(screenshotPath, Buffer.from(screenshot.data, 'base64'))
    await preserveVisualArtifact(screenshotPath, `${visualArtifactPrefix}-final.png`)
    const traceLevelScreenshotPaths = []
    for (const level of requiredTraceLevels) {
      const traceLevelScreenshotPath = await captureTraceLevelScreenshot(cdp, root, visualArtifactPrefix, level)
      if (traceLevelScreenshotPath) traceLevelScreenshotPaths.push(traceLevelScreenshotPath)
    }
    logStep('browser_html_final_screenshot_ok', {
      partialScreenshotPath,
      screenshotPath,
      traceLevelScreenshotPaths,
      partialOutput: partialState?.output ?? null,
      status: finalState.status,
      traceCount: finalState.traceCount,
      traceLevels: finalState.traceLevels,
      traceAliases: finalState.traceAliases,
    })
  } finally {
    await cdp?.close?.().catch(() => {})
    await stopProcess(chrome)
  }
}

export async function captureTraceLevelScreenshot(cdp, root, visualArtifactPrefix, level) {
  const evaluated = await cdp.send('Runtime.evaluate', {
    returnByValue: true,
    expression: `(() => {
      const items = Array.from(document.querySelectorAll('#trace-feed .trace-item'));
      const item = items.find((candidate) => Array.from(candidate.querySelectorAll('.trace-meta span')).some((span) => (span.textContent || '').trim() === ${JSON.stringify(level)}));
      if (!item) return false;
      item.scrollIntoView({ block: 'center', inline: 'nearest' });
      item.style.outline = '3px solid #1d4ed8';
      item.style.outlineOffset = '2px';
      return true;
    })()`,
  })
  if (!evaluated.result?.value) return null
  await new Promise((resolve) => setTimeout(resolve, 250))
  const screenshot = await captureBrowserScreenshot(cdp, `browser HTML ${level} trace screenshot`)
  if (typeof screenshot.data !== 'string' || screenshot.data.length < 1000) return null
  const screenshotPath = path.join(root, `${visualArtifactPrefix}-${level}.png`)
  await writeFile(screenshotPath, Buffer.from(screenshot.data, 'base64'))
  await preserveVisualArtifact(screenshotPath, `${visualArtifactPrefix}-${level}.png`)
  return screenshotPath
}

export async function captureBrowserScreenshot(cdp, label) {
  return await withTimeout(
    cdp.send('Page.captureScreenshot', { format: 'png', fromSurface: true }),
    10_000,
    label,
  )
}

export async function preserveVisualArtifact(sourcePath, fileName) {
  const targetRoot = process.env.ARROBA_PUBLICATION_VISUAL_ARTIFACTS_DIR
  if (!targetRoot) return
  await mkdir(targetRoot, { recursive: true })
  await cp(sourcePath, path.join(targetRoot, fileName), { force: true })
}

export async function waitForBrowserHtmlPartialOutput(cdp, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  let lastState = null
  while (Date.now() < deadline) {
    const evaluated = await withTimeout(cdp.send('Runtime.evaluate', {
      returnByValue: true,
      expression: `(() => {
        const status = document.querySelector('#status')?.textContent?.trim() || '';
        const output = document.querySelector('#output')?.textContent?.trim() || '';
        const iframe = document.querySelector('#html-output iframe');
        const traceCount = document.querySelectorAll('#trace-feed .trace-item').length;
        return {
          status,
          output,
          hasHtmlFinal: !!iframe,
          traceCount,
          ok: output.includes('"value":1841') && !iframe && traceCount > 0,
        };
      })()`,
    }), 3_000, 'browser HTML partial Runtime.evaluate')
    lastState = evaluated.result?.value ?? null
    if (lastState?.ok) return lastState
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`browser did not render HTML workflow partial output before final dashboard: ${JSON.stringify(lastState)}`)
}

export async function waitForBrowserHtmlFinalOutput(
  cdp,
  {
    timeoutMs = 30_000,
    expectedHtmlText = 'Vibrant Workflow Dashboard',
    requiredHtmlSnippets = [],
    requiredTraceLevels = [],
    requiredTraceAlias = null,
  } = {},
) {
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
          meta: Array.from(item.querySelectorAll('.trace-meta span')).map((span) => span.textContent || ''),
        }));
        const traceCount = traces.length;
        const requiredLevels = ${JSON.stringify(requiredTraceLevels)};
        const requiredAlias = ${JSON.stringify(requiredTraceAlias)};
        const requiredHtmlSnippets = ${JSON.stringify(requiredHtmlSnippets)};
        const traceLevelSet = new Set(traces.map((trace) => trace.meta[2]).filter(Boolean));
        const traceAliasSet = new Set(traces.map((trace) => trace.meta[0]).filter(Boolean));
        const missingTraceLevels = requiredLevels.filter((level) => !traceLevelSet.has(level));
        const levelsOk = missingTraceLevels.length === 0;
        const aliasOk = !requiredAlias || traceAliasSet.has(requiredAlias);
        const htmlOk = iframeSrcdoc.includes(${JSON.stringify(expectedHtmlText)})
          && requiredHtmlSnippets.every((snippet) => iframeSrcdoc.includes(snippet));
        return {
          status,
          iframeSrcdoc,
          traceCount,
          traceLevels: Array.from(traceLevelSet).sort(),
          traceAliases: Array.from(traceAliasSet).sort(),
          missingTraceLevels,
          htmlOk,
          aliasOk,
          ok: status === 'Completed'
            && htmlOk
            && traceCount > 0
            && levelsOk
            && aliasOk,
        };
      })()`,
    }), 3_000, 'browser HTML final Runtime.evaluate')
    lastState = evaluated.result?.value ?? null
    if (lastState?.ok) return lastState
    if (
      lastState?.status === 'Completed'
      && lastState?.htmlOk
      && lastState?.aliasOk
      && Array.isArray(lastState?.missingTraceLevels)
      && lastState.missingTraceLevels.length > 0
    ) {
      throw new Error(`browser completed without required trace levels: ${JSON.stringify(lastState)}`)
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  throw new Error(`browser did not render HTML final workflow output: ${JSON.stringify(lastState)}`)
}

export async function runHumanHttpRootFormBrowserDrill({ baseUrl, root, timeoutMs = 30_000 }) {
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
    await preserveVisualArtifact(screenshotPath, 'local-human-http-root-form-final.png')
    logStep('browser_root_form_screenshot_ok', { screenshotPath, statuses, outputs })
  } finally {
    await cdp?.close?.().catch(() => {})
    await stopProcess(chrome)
  }
}

export async function runSharedViewerBrowserDrill({ baseUrl, root, label, prompt, timeoutMs = 30_000 }) {
  const chromePath = await findChromeExecutable()
  if (!chromePath) {
    logStep(`${label}_browser_viewer_skipped`, { reason: 'chrome-not-found' })
    return
  }
  const safeLabel = label.replace(/[^a-z0-9-]/gi, '-').toLowerCase()
  const debuggingPort = await freePort()
  const userDataDir = path.join(root, `chrome-${safeLabel}-profile`)
  const screenshotPath = path.join(root, `${safeLabel}-viewer-final.png`)
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
    `chrome-${safeLabel}-publication`,
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
    const submitted = await cdp.send('Runtime.evaluate', {
      returnByValue: true,
      expression: `(() => {
        const form = document.querySelector('#invoke-form');
        const prompt = form?.querySelector('[name="prompt"]');
        if (!form || !prompt) return false;
        prompt.value = ${JSON.stringify(prompt)};
        form.requestSubmit();
        return true;
      })()`,
    })
    if (submitted.result?.value !== true) {
      throw new Error(`${label} browser viewer could not be submitted`)
    }
    const finalState = await waitForBrowserFinalOutput(cdp, timeoutMs)
    const statuses = finalState.statuses ?? []
    const outputs = finalState.outputs ?? []
    if (finalState.status !== 'Completed' && !statuses.includes('Completed')) {
      throw new Error(`${label} browser viewer did not complete; state=${JSON.stringify(finalState)}`)
    }
    for (const expectedValue of ['"value":1841', '"value":1842']) {
      if (!outputs.some((output) => output.includes(expectedValue)) && !String(finalState.output ?? '').includes(expectedValue)) {
        throw new Error(`${label} browser viewer did not observe ${expectedValue} output; outputs=${JSON.stringify(outputs)}, state=${JSON.stringify(finalState)}`)
      }
    }
    const screenshot = await cdp.send('Page.captureScreenshot', { format: 'png', fromSurface: true })
    if (typeof screenshot.data !== 'string' || screenshot.data.length < 1000) {
      throw new Error(`${label} browser viewer screenshot was empty`)
    }
    await writeFile(screenshotPath, Buffer.from(screenshot.data, 'base64'))
    await preserveVisualArtifact(screenshotPath, `${safeLabel}-viewer-final.png`)
    logStep(`${label}_browser_viewer_ok`, { screenshotPath, statuses, outputs })
  } finally {
    await cdp?.close?.().catch(() => {})
    await stopProcess(chrome)
  }
}

export async function waitForBrowserRootForm(cdp) {
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

export async function waitForChromeTarget(debuggingPort, expectedUrl, chrome) {
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

export async function waitForBrowserFinalOutput(cdp, timeoutMs = 30_000) {
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

export function browserStatusRecorderScript() {
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
