import assert from 'node:assert/strict'
import test from 'node:test'

import { HUMAN_HTTP_COMPOSER_METHODS, createSchedulePublicationArtifacts, publicationRequestTransportOptions } from './live-workflow-publication-drill-session-builders.mjs'

test('publication drill human HTTP composer publications allow GET and POST', () => {
  assert.deepEqual(HUMAN_HTTP_COMPOSER_METHODS, ['GET', 'POST'])
  assert.deepEqual(publicationRequestTransportOptions({
    route: '/final/*',
    methods: HUMAN_HTTP_COMPOSER_METHODS,
    transportKind: 'human_http',
  }), {
    route: '/final/*',
    methods: ['GET', 'POST'],
    transport: { kind: 'human_http' },
    parser: { kind: 'json' },
    mode: 'async',
  })
})

test('publication drill omits HTTP-only overrides for WebSocket transport', () => {
  assert.deepEqual(publicationRequestTransportOptions({
    route: '/socket',
    methods: ['GET'],
    transportKind: 'websocket_json',
  }), {
    route: '/socket',
    transport: { kind: 'websocket_json' },
    mode: 'async',
  })
})

test('publication drill preserves API SSE HTTP input options', () => {
  assert.deepEqual(publicationRequestTransportOptions({
    route: '/invoke',
    methods: ['POST'],
    transportKind: 'api_sse_json',
  }), {
    route: '/invoke',
    methods: ['POST'],
    transport: { kind: 'api_sse_json' },
    parser: { kind: 'json' },
    mode: 'async',
  })
})

test('publication drill lets MCP read input from tool arguments', () => {
  assert.deepEqual(publicationRequestTransportOptions({
    route: '/mcp',
    methods: ['POST'],
    transportKind: 'mcp',
  }), {
    route: '/mcp',
    methods: ['POST'],
    transport: { kind: 'mcp' },
    mode: 'sync',
  })
})

test('schedule publication freezes a snapshot only after its schedule exists', async () => {
  const requests = []
  const client = {
    async send(request) {
      requests.push(request)
      if (request.CreateWorkflowSchedule) {
        return { WorkflowScheduleCreated: { schedule: { id: 'schedule-1' } } }
      }
      if (request.CreateWorkflowPublication) {
        return { WorkflowPublicationCreated: { publication: { id: 'publication-1' } } }
      }
      throw new Error(`unexpected request: ${JSON.stringify(request)}`)
    },
  }

  const artifacts = await createSchedulePublicationArtifacts(client, {
    sessionId: 'session-1',
    workflowId: 'workflow-1',
    endpointId: 'endpoint-1',
  })

  assert.deepEqual(artifacts, {
    schedule: { id: 'schedule-1' },
    publication: { id: 'publication-1' },
  })
  assert.ok(requests[0].CreateWorkflowSchedule)
  assert.ok(requests[1].CreateWorkflowPublication)
  assert.equal(requests[0].CreateWorkflowSchedule.trigger.every_seconds, 60)
})
