import assert from 'node:assert/strict'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'

import {
  HUMAN_HTTP_FORM_INVOKE_PATH,
  assertDeployedWorkflowViewerFormPage,
  copyCloudProfile,
  deployedWorkflowFormInvokeRequest,
  deployedWorkflowFormResultEventsPath,
  validateTransport,
} from './live-cloud-publication-deployment-drill-transport.mjs'

test('cloud relay drill profile is copied into the isolated kernel home', async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'chariox-cloud-profile-'))
  t.after(() => rm(root, { recursive: true, force: true }))
  const source = path.join(root, 'source.json')
  const charioxHome = path.join(root, 'chariox-home')
  const payload = '{"cloud_relay":{"api_url":"https://cloud.example"}}\n'
  await writeFile(source, payload, 'utf8')

  await copyCloudProfile(charioxHome, source)

  assert.equal(await readFile(path.join(charioxHome, 'daemon', 'config.json'), 'utf8'), payload)
})

test('human HTTP form invoke requests target the fixed publication form endpoint', () => {
  assert.deepEqual(deployedWorkflowFormInvokeRequest('review the exact head'), {
    path: HUMAN_HTTP_FORM_INVOKE_PATH,
    method: 'POST',
    headers: { accept: 'text/html', 'content-type': 'application/json' },
    body: JSON.stringify({ prompt: 'review the exact head' }),
  })
  assert.throws(() => deployedWorkflowFormInvokeRequest('   '), /non-empty prompt/)
  assert.throws(() => deployedWorkflowFormInvokeRequest(undefined), /non-empty prompt/)
})

test('human HTTP viewer form assertion requires form, prompt field, and invoke endpoint', () => {
  const page = [
    '<!doctype html>',
    `<script>window.__charioxPublicationViewerConfig = {"humanFormInvokePath":"${HUMAN_HTTP_FORM_INVOKE_PATH}"};</script>`,
    '<form id="invoke-form"><textarea name="prompt"></textarea></form>',
  ].join('')
  assertDeployedWorkflowViewerFormPage(page, 'fixture viewer')

  assert.throws(
    () => assertDeployedWorkflowViewerFormPage(page.replace('<form id="invoke-form"', '<form>'), 'fixture viewer'),
    /omitted the prompt invoke form/,
  )
  assert.throws(
    () => assertDeployedWorkflowViewerFormPage(page.replace('name="prompt"', 'name="q"'), 'fixture viewer'),
    /omitted the prompt field/,
  )
  assert.throws(
    () => assertDeployedWorkflowViewerFormPage(page.replace(HUMAN_HTTP_FORM_INVOKE_PATH, '/invoke'), 'fixture viewer'),
    /did not configure the .* form endpoint/,
  )
})

test('human HTTP form result event paths require a configured stream URL', () => {
  const resultPage = (eventsUrl) => (
    `<script>window.__charioxPublicationViewerConfig = {"eventsUrl":"${eventsUrl}"};</script>`
  )
  assert.equal(
    deployedWorkflowFormResultEventsPath(
      resultPage('/.well-known/chariox/publication/invocations/req_1/events'),
    ),
    '/.well-known/chariox/publication/invocations/req_1/events',
  )
  assert.equal(
    deployedWorkflowFormResultEventsPath(
      resultPage('/.well-known/chariox/publication/runs/run_2/events'),
    ),
    '/.well-known/chariox/publication/runs/run_2/events',
  )
  assert.throws(
    () => deployedWorkflowFormResultEventsPath('<html>no config</html>'),
    /did not expose an event stream URL/,
  )
})

test('human HTTP transport writes viewer and completed SSE evidence', async (t) => {
  const artifactsDir = await mkdtemp(path.join(os.tmpdir(), 'chariox-publication-transport-'))
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
    if (request.url === '/publication/invocations/req_fixture/events') {
      let body = ''
      request.on('data', (chunk) => { body += chunk })
      request.on('end', () => {
        assert.equal(body, '')
        response.writeHead(200, { 'content-type': 'text/event-stream' })
        response.end([
          'event: trace',
          'data: {"message":"form fixture trace"}',
          '',
          'event: final',
          'data: {"workflow_run":{"status":"Completed","failure_events":[]}}',
          '',
        ].join('\n'))
      })
      return
    }
    if (request.url === `/publication${HUMAN_HTTP_FORM_INVOKE_PATH}`) {
      let body = ''
      request.on('data', (chunk) => { body += chunk })
      request.on('end', () => {
        assert.equal(request.headers['content-type'], 'application/json')
        assert.deepEqual(JSON.parse(body), { prompt: 'fixture prompt' })
        response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
        response.end([
          '<!doctype html><form id="invoke-form"><textarea name="prompt"></textarea></form>',
          `<script>window.__charioxPublicationViewerConfig =`,
          `{"humanFormInvokePath":"${HUMAN_HTTP_FORM_INVOKE_PATH}",`,
          `"eventsUrl":"/invocations/req_fixture/events"};</script>`,
          '<div id="output">fixture result</div>',
        ].join(''))
      })
      return
    }
    response.writeHead(200, { 'content-type': 'text/html' })
    response.end([
      '<!doctype html><form id="invoke-form"><textarea name="prompt"></textarea></form>',
      `<script>window.__charioxPublicationViewerConfig =`,
      `{"humanFormInvokePath":"${HUMAN_HTTP_FORM_INVOKE_PATH}","eventsUrl":"/events"};`,
      'const stream = new EventSource("/events");</script>',
    ].join(''))
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
  assert.equal(evidence.formPost.eventsScope, 'invocation')
  assert.equal(evidence.formPost.streamedFinal, true)
  const formEvidence = JSON.parse(await readFile(evidence.formPost.evidencePath, 'utf8'))
  assert.deepEqual(formEvidence.request.body, { prompt: 'fixture prompt' })
  assert.equal(formEvidence.request.path, HUMAN_HTTP_FORM_INVOKE_PATH)
  assert.equal(formEvidence.result.eventsScope, 'invocation')
  assert.match(await readFile(evidence.formPost.transcriptPath, 'utf8'), /event: final/)
})
