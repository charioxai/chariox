import assert from 'node:assert/strict'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'

import { validateTransport } from './live-cloud-publication-deployment-drill-transport.mjs'

test('human HTTP transport writes viewer and completed SSE evidence', async (t) => {
  const artifactsDir = await mkdtemp(path.join(os.tmpdir(), 'arroba-publication-transport-'))
  const server = http.createServer((request, response) => {
    if (request.url === '/publication/events') {
      response.writeHead(200, { 'content-type': 'text/event-stream' })
      response.end([
        'event: trace',
        'data: {"message":"fixture trace"}',
        '',
        'event: final',
        'data: {"workflow_run":{"status":"Completed","failure_events":[]}}',
        '',
      ].join('\n'))
      return
    }
    response.writeHead(200, { 'content-type': 'text/html' })
    response.end('<!doctype html><script>window.__arrobaPublicationViewerConfig = {"eventsUrl":"/events"}; const stream = new EventSource("/events");</script>')
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  t.after(async () => {
    await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()))
    await rm(artifactsDir, { recursive: true, force: true })
  })

  const address = server.address()
  assert(address && typeof address === 'object')
  const evidence = await validateTransport({
    transport: 'human_http',
    publicBaseUrl: `http://127.0.0.1:${address.port}/publication`,
    prompt: 'fixture prompt',
    artifactsDir,
    slug: 'fixture',
    expectHtmlDashboard: false,
    expectAgentAppShopping: false,
    browserScreenshot: false,
  })

  assert.match(await readFile(evidence.htmlPath, 'utf8'), /EventSource/)
  assert.match(await readFile(evidence.transcriptPath, 'utf8'), /event: final/)
})
