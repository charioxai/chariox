import assert from 'node:assert/strict'
import test from 'node:test'

import {
  assertDrillCLiveObserverManifest,
  assertDrillCSharedRoomEvidence,
  captureDrillCRoomCheckpoint,
} from './tui-web-parity-visual-control.mjs'

const sessionId = 'room-session-live'
const environmentId = 'environment-live'
const sliceId = 'slice-live'
const viewport = {
  css_width: 1280,
  css_height: 800,
  device_scale_factor: 1,
  desktop_pixel_width: 1280,
  desktop_pixel_height: 800,
  revision: 4,
  last_actor_id: null,
}
const sliceBinding = {
  session_id: sessionId,
  slice_id: sliceId,
  owner_kernel_id: 'kernel-home',
  worker_kernel_ref: 'worker-live',
}
const resourceInventory = {
  session_id: sessionId,
  environment_id: environmentId,
  slice_id: sliceId,
  browser_ids: ['browser-live'],
  profile_ids: ['profile-live'],
}
const baseline = {
  capturedAt: '2026-09-23T10:00:00.000Z',
  sessionId,
  environment: {
    session_id: sessionId,
    environment_id: environmentId,
    runtime_generation: 8,
    focused_tab_id: 'tab-live',
    viewport,
    tabs: [{ tab_id: 'tab-live', url: 'https://fixture.invalid/page', title: 'Shared page', document_revision: 5 }],
  },
  sliceBinding,
  resourceInventory,
  highestActionSequence: 41,
}
const computerAction = {
  action_id: 'action-computer',
  actor_id: 'agent:agent-live',
  sequence: 42,
  mode: 'computer',
  kind: 'keyboard_text',
  state: 'completed',
  submitted_at_ms: 1000,
  targets: [{ kind: 'desktop' }],
}
const webTakeover = {
  action_id: 'action-web-human',
  actor_id: 'user:local',
  sequence: 43,
  mode: 'computer',
  kind: 'pointer_click',
  state: 'completed',
  submitted_at_ms: 1100,
  targets: [{ kind: 'desktop' }],
}
const checkpoint = {
  capturedAt: '2026-09-23T10:00:05.000Z',
  sessionId,
  environment: {
    session_id: sessionId,
    environment_id: environmentId,
    lifecycle: 'ready',
    runtime_generation: 8,
    focused_tab_id: 'tab-live',
    viewport,
    tabs: [{ tab_id: 'tab-live', url: 'https://fixture.invalid/page', title: 'Shared page', document_revision: 6 }],
    actors: [
      { actor_id: 'agent:agent-live', kind: 'agent', presence: 'present' },
      { actor_id: 'user:local', kind: 'human', presence: 'present' },
    ],
  },
  sliceBinding,
  resourceInventory,
  actions: [webTakeover, computerAction],
}
const display = {
  environmentId,
  sliceId,
  runtimeGeneration: 8,
  viewport,
  browserIds: ['browser-live'],
  profileIds: ['profile-live'],
}
const web = {
  schema: 'chariox.browser_computer.drill_c.web_observer.v1',
  status: 'observed',
  observer: 'web',
  client: 'production-local-web-view',
  observedAt: '2026-09-23T10:00:04.000Z',
  sessionId,
  environmentId,
  sliceId,
  runtimeGeneration: 8,
  display,
  browser: {
    focusedTabId: 'tab-live',
    tab: {
      tabId: 'tab-live',
      url: 'https://fixture.invalid/page',
      title: 'Shared page',
      documentRevision: 6,
    },
  },
  actions: {
    computer: actionIdentity(computerAction),
    webTakeover: actionIdentity(webTakeover),
  },
}
const tui = {
  sessionId,
  statusNotice: [
    `Room environment ${environmentId}`,
    'lifecycle=ready generation=8 cursor=55',
    'health=browser:ready, desktop:ready',
    'viewport=1280x800 css=1280x800 scale=1 revision=4',
    'tab=tab-live Shared page — https://fixture.invalid/page',
  ].join('\n'),
  actionsNotice: [
    'Room actions (2)',
    '#42 action-computer actor=agent:agent-live computer:keyboard_text target=desktop state=completed submitted_at_ms=1000',
    '#43 action-web-human actor=user:local computer:pointer_click target=desktop state=completed submitted_at_ms=1100',
    'next_before=none',
  ].join('\n'),
}

test('Drill C verifier accepts matching live kernel, TUI, and Web observations', () => {
  const report = assertDrillCSharedRoomEvidence({ baseline, checkpoint, web, tui })

  assert.equal(report.status, 'passed')
  assert.equal(report.sameRoom, true)
  assert.equal(report.sameEnvironment, true)
  assert.equal(report.sameTab, true)
  assert.equal(report.sameDisplay, true)
  assert.equal(report.actions.computer.actionId, computerAction.action_id)
  assert.equal(report.actions.webTakeover.actorId, webTakeover.actor_id)
})

test('Drill C verifier rejects a dev-stub visual manifest', () => {
  assert.throws(() => assertDrillCLiveObserverManifest({
    schema: 'chariox.tui_web_parity_visual_session.v1',
    sessionId,
    sliceId,
    baseline,
    webObservationPath: '/tmp/web.json',
  }), /live Room observer manifest/)
})

test('Drill C verifier accepts only a captured live observer baseline', () => {
  assert.doesNotThrow(() => assertDrillCLiveObserverManifest({
    schema: 'chariox.drill_c.live_observer_session.v1',
    kernelUrl: 'ws://127.0.0.1:43118/kernel',
    sessionId,
    sliceId,
    baseline,
    webObservationPath: '/tmp/web-observer.json',
  }))
})

test('Room checkpoint reads identity and action history from public kernel requests', async () => {
  const requests = {
    getRoomEnvironmentStateRequest: (id) => ({ GetRoomEnvironmentState: { session_id: id } }),
    getRoomEnvironmentSliceRequest: (id) => ({ GetRoomEnvironmentSlice: { session_id: id } }),
    getRoomEnvironmentResourceInventoryRequest: (roomId, currentSliceId) => ({
      GetRoomEnvironmentResourceInventory: { session_id: roomId, slice_id: currentSliceId },
    }),
    listRoomEnvironmentActionHistoryRequest: (id, beforeSequence, limit) => ({
      ListRoomEnvironmentActionHistory: { session_id: id, before_sequence: beforeSequence, limit },
    }),
  }
  const seen = []
  const client = {
    async send(request) {
      seen.push(request)
      if ('GetRoomEnvironmentState' in request) return { RoomEnvironmentState: { environment: checkpoint.environment } }
      if ('GetRoomEnvironmentSlice' in request) return { RoomEnvironmentSlice: { binding: sliceBinding } }
      if ('GetRoomEnvironmentResourceInventory' in request) return { RoomEnvironmentResourceInventory: { inventory: resourceInventory } }
      if ('ListRoomEnvironmentActionHistory' in request) {
        return { RoomEnvironmentActionHistoryListed: { page: { actions: [webTakeover, computerAction] } } }
      }
      throw new Error('unexpected kernel request')
    },
  }

  const captured = await captureDrillCRoomCheckpoint({ client, requests, sessionId, sliceId })

  assert.equal(captured.sessionId, sessionId)
  assert.equal(captured.highestActionSequence, 43)
  assert.deepEqual(captured.actions, [webTakeover, computerAction])
  assert.deepEqual(seen.map((request) => Object.keys(request)[0]), [
    'GetRoomEnvironmentState',
    'GetRoomEnvironmentSlice',
    'GetRoomEnvironmentResourceInventory',
    'ListRoomEnvironmentActionHistory',
  ])
})

test('Drill C verifier rejects Web evidence with a different tab or viewport', () => {
  assert.throws(() => assertDrillCSharedRoomEvidence({
    baseline,
    checkpoint,
    web: { ...web, browser: { ...web.browser, focusedTabId: 'other-tab' } },
    tui,
  }), /different focused tab/)
  assert.throws(() => assertDrillCSharedRoomEvidence({
    baseline,
    checkpoint,
    web: { ...web, display: { ...display, viewport: { ...viewport, desktop_pixel_width: 1024 } } },
    tui,
  }), /different canonical display/)
})

test('Drill C verifier rejects missing Computer work and mismatched actor history', () => {
  assert.throws(() => assertDrillCSharedRoomEvidence({
    baseline,
    checkpoint: { ...checkpoint, actions: [webTakeover] },
    web,
    tui,
  }), /no completed agent Computer work/)
  assert.throws(() => assertDrillCSharedRoomEvidence({
    baseline,
    checkpoint,
    web: { ...web, actions: { ...web.actions, webTakeover: { ...web.actions.webTakeover, actorId: 'agent:wrong' } } },
    tui,
  }), /Action identity differs from kernel history/)
})

function actionIdentity(action) {
  return {
    actionId: action.action_id,
    actorId: action.actor_id,
    sequence: action.sequence,
    mode: action.mode,
    kind: action.kind,
    state: action.state,
  }
}
