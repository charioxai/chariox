import assert from 'node:assert/strict'
import test from 'node:test'
import vm from 'node:vm'

import { browserStatusRecorderScript, captureBrowserScreenshot, waitForBrowserHtmlFinalOutput } from './live-workflow-publication-drill-browser.mjs'

test('browser publication recorder observes output changes without a status change', () => {
  const observers = new Map()
  const status = { textContent: 'Running' }
  const output = { textContent: '' }
  class MutationObserver {
    constructor(callback) {
      this.callback = callback
    }

    observe(target) {
      const callbacks = observers.get(target) ?? []
      callbacks.push(this.callback)
      observers.set(target, callbacks)
    }
  }
  const window = { EventSource: undefined }
  const document = {
    readyState: 'complete',
    querySelector(selector) {
      if (selector === '#status') return status
      if (selector === '#output') return output
      return null
    },
  }

  vm.runInNewContext(browserStatusRecorderScript(), { document, MutationObserver, window })
  output.textContent = '{"value":1841}'
  for (const callback of observers.get(output) ?? []) callback()

  assert.deepEqual([...window.__arrobaPublicationDrillOutputs], ['{"value":1841}'])
})

test('HTML final waiter verifies the sandboxed frame has rendered before capture', async () => {
  const calls = []
  const frameCdp = {
    async send(method, params = {}) {
      calls.push({ target: 'frame', method, params })
      if (method === 'Runtime.enable') return {}
      if (method === 'Runtime.evaluate' && params.awaitPromise) return { result: { value: true } }
      if (method === 'Runtime.evaluate') {
        return {
          result: {
            value: {
              readyState: 'complete',
              text: 'Vibrant Workflow Dashboard Revenue pulse',
              outerHtml: '<main data-dashboard="ready">Vibrant Workflow Dashboard</main>',
              width: 900,
              height: 800,
            },
          },
        }
      }
      throw new Error(`unexpected frame CDP method ${method}`)
    },
    async close() {
      calls.push({ target: 'frame', method: 'close', params: {} })
    },
  }
  const cdp = {
    async connectChildTarget(selector) {
      calls.push({ target: 'page', method: 'connectChildTarget', params: selector })
      return frameCdp
    },
    async send(method, params = {}) {
      calls.push({ target: 'page', method, params })
      if (method === 'Runtime.evaluate') {
        return {
          result: {
            value: {
              status: 'Completed',
              iframeSrcdoc: '<main data-dashboard="ready">Vibrant Workflow Dashboard</main>',
              traceCount: 1,
              traceLevels: [],
              traceAliases: [],
              missingTraceLevels: [],
              htmlOk: true,
              aliasOk: true,
              ok: true,
            },
          },
        }
      }
      throw new Error(`unexpected CDP method ${method}`)
    },
  }

  const result = await waitForBrowserHtmlFinalOutput(cdp, {
    timeoutMs: 100,
    expectedHtmlText: 'Vibrant Workflow Dashboard',
    requiredHtmlSnippets: ['data-dashboard="ready"'],
  })

  assert.equal(result.renderedFrame.readyState, 'complete')
  assert.equal(result.renderedFrame.width, 900)
  assert.ok(calls.some(({ method }) => method === 'connectChildTarget'))
  assert.ok(calls.some(({ target, method, params }) => target === 'frame' && method === 'Runtime.evaluate' && params.awaitPromise))
  assert.ok(calls.some(({ target, method }) => target === 'frame' && method === 'close'))
})

test('browser screenshot waits for the document to paint before capture', async () => {
  const calls = []
  const cdp = {
    async send(method, params = {}) {
      calls.push({ method, params })
      if (method === 'Runtime.evaluate') return { result: { value: true } }
      if (method === 'Page.captureScreenshot') return { data: 'png-data' }
      throw new Error(`unexpected CDP method ${method}`)
    },
  }

  const screenshot = await captureBrowserScreenshot(cdp, 'paint-order regression')

  assert.equal(screenshot.data, 'png-data')
  assert.deepEqual(calls.map(({ method }) => method), ['Runtime.evaluate', 'Page.captureScreenshot'])
  assert.equal(calls[0].params.awaitPromise, true)
})
