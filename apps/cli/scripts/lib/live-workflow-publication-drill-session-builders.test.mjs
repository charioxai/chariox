import assert from 'node:assert/strict'
import test from 'node:test'

import { publicationRequestTransportOptions } from './live-workflow-publication-drill-session-builders.mjs'

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
