import assert from "node:assert/strict"
import test from "node:test"

import type { RoomEnvironmentSnapshot } from "@chariox/kernel-client/kernel-types"

import { parseSlashCommand } from "./commands.js"
import { handleRoomSlashCommand } from "./room-command-handler.js"

test("/room status reads and renders the attached Room environment", async () => {
  const requests: unknown[] = []
  const notices: string[] = []
  const command = parseSlashCommand("/room status")
  assert.equal(command?.kind, "room")

  await handleRoomSlashCommand({
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return { RoomEnvironmentState: { environment: roomEnvironment() } } as TResponse
    },
    appendNotice: (notice) => notices.push(notice),
    flashFooter: () => undefined,
  }, command)

  assert.deepEqual(requests, [{ GetRoomEnvironmentState: { session_id: "session-1" } }])
  assert.deepEqual(notices, [[
    "Room environment environment-1",
    "lifecycle=ready generation=2 cursor=4",
    "health=browser_controller:ready, browser:ready, desktop:ready, streamer:ready",
    "viewport=1280x720 css=1280x720 scale=1 revision=3",
    "tab=tab-1 Docs — https://example.test/docs",
    "actors=Mara (agent,present), Miguel (human,present)",
    "input=desktop:Mara",
    "last_action=none",
  ].join("\n")])
})

test("/room start uses the portable default viewport and renders the authoritative result", async () => {
  const requests: unknown[] = []
  const notices: string[] = []
  const command = parseSlashCommand("/room start")
  assert.equal(command?.kind, "room")

  await handleRoomSlashCommand({
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return {
        RoomEnvironmentUpdated: {
          environment: { ...roomEnvironment(), lifecycle: "starting" },
        },
      } as TResponse
    },
    appendNotice: (notice) => notices.push(notice),
    flashFooter: () => undefined,
  }, command)

  assert.deepEqual(requests, [{
    StartRoomEnvironment: {
      session_id: "session-1",
      viewport: {
        css_width: 1280,
        css_height: 800,
        device_scale_factor: 1,
        desktop_pixel_width: 1280,
        desktop_pixel_height: 800,
      },
    },
  }])
  assert.match(notices[0] ?? "", /^Room environment environment-1\nlifecycle=starting/)
})

test("/room start accepts an explicit CSS viewport and scale", async () => {
  const requests: unknown[] = []
  const command = parseSlashCommand("/room start 1440x900 2")
  assert.equal(command?.kind, "room")

  await handleRoomSlashCommand({
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return { RoomEnvironmentUpdated: { environment: roomEnvironment() } } as TResponse
    },
    appendNotice: () => undefined,
    flashFooter: () => undefined,
  }, command)

  assert.deepEqual(requests, [{
    StartRoomEnvironment: {
      session_id: "session-1",
      viewport: {
        css_width: 1440,
        css_height: 900,
        device_scale_factor: 2,
        desktop_pixel_width: 2880,
        desktop_pixel_height: 1800,
      },
    },
  }])
})

test("/room stop and retry use the existing kernel lifecycle requests", async () => {
  const requests: unknown[] = []
  const notices: string[] = []
  const deps = {
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      const lifecycle = Object.prototype.hasOwnProperty.call(request, "StopRoomEnvironment")
        ? "stopped"
        : "starting"
      return {
        RoomEnvironmentUpdated: {
          environment: { ...roomEnvironment(), lifecycle },
        },
      } as TResponse
    },
    appendNotice: (notice: string) => notices.push(notice),
    flashFooter: () => undefined,
  }
  const stop = parseSlashCommand("/room stop")
  const retry = parseSlashCommand("/room retry")
  assert.equal(stop?.kind, "room")
  assert.equal(retry?.kind, "room")

  await handleRoomSlashCommand(deps, stop)
  await handleRoomSlashCommand(deps, retry)

  assert.deepEqual(requests, [
    { StopRoomEnvironment: { session_id: "session-1" } },
    { RetryRoomEnvironment: { session_id: "session-1" } },
  ])
  assert.match(notices[0] ?? "", /^Room environment environment-1\nlifecycle=stopped/)
  assert.match(notices[1] ?? "", /^Room environment environment-1\nlifecycle=starting/)
})

test("/room lifecycle commands reject invalid arguments without reaching the kernel", async () => {
  const requests: unknown[] = []
  const flashes: string[] = []
  const deps = {
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return {} as TResponse
    },
    appendNotice: () => undefined,
    flashFooter: (message: string) => flashes.push(message),
  }
  const invalidStart = parseSlashCommand("/room start wide")
  const fractionalScale = parseSlashCommand("/room start 1280x800 1.5")
  const invalidStop = parseSlashCommand("/room stop now")
  assert.equal(invalidStart?.kind, "room")
  assert.equal(fractionalScale?.kind, "room")
  assert.equal(invalidStop?.kind, "room")

  await handleRoomSlashCommand(deps, invalidStart)
  await handleRoomSlashCommand(deps, fractionalScale)
  await handleRoomSlashCommand(deps, invalidStop)

  assert.deepEqual(requests, [])
  assert.deepEqual(flashes, [
    "usage: /room start [WIDTHxHEIGHT] [SCALE]",
    "usage: /room start [WIDTHxHEIGHT] [SCALE]",
    "usage: /room status|start [WIDTHxHEIGHT] [SCALE]|stop|retry|takeover|release [desktop|tab TAB_ID]|cancel ACTION_ID",
  ])
})

test("/room takeover and release use authenticated kernel input requests", async () => {
  const requests: unknown[] = []
  const notices: string[] = []
  const deps = {
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      if (Object.prototype.hasOwnProperty.call(request, "ReleaseRoomEnvironmentInput")) {
        return {
          RoomEnvironmentInputReleased: {
            environment: { ...roomEnvironment(), input_ownership: [] },
          },
        } as TResponse
      }
      const target = (request as {
        RequestRoomEnvironmentInputTakeover: { target: { kind: string } }
      }).RequestRoomEnvironmentInputTakeover.target
      return {
        RoomEnvironmentTakeoverUpdated: target.kind === "desktop"
          ? {
              outcome: { state: "granted" },
              environment: {
                ...roomEnvironment(),
                input_ownership: [{ target: { kind: "desktop" }, actor_id: "user:miguel" }],
              },
            }
          : {
              outcome: { state: "cancellation_required", action_ids: ["action-7"] },
              environment: roomEnvironment(),
            },
      } as TResponse
    },
    appendNotice: (notice: string) => notices.push(notice),
    flashFooter: () => undefined,
  }
  const takeoverDesktop = parseSlashCommand("/room takeover")
  const takeoverTab = parseSlashCommand("/room takeover tab tab-1")
  const releaseTab = parseSlashCommand("/room release tab tab-1")
  assert.equal(takeoverDesktop?.kind, "room")
  assert.equal(takeoverTab?.kind, "room")
  assert.equal(releaseTab?.kind, "room")

  await handleRoomSlashCommand(deps, takeoverDesktop)
  await handleRoomSlashCommand(deps, takeoverTab)
  await handleRoomSlashCommand(deps, releaseTab)

  assert.deepEqual(requests, [
    {
      RequestRoomEnvironmentInputTakeover: {
        session_id: "session-1",
        target: { kind: "desktop" },
      },
    },
    {
      RequestRoomEnvironmentInputTakeover: {
        session_id: "session-1",
        target: { kind: "browser_tab", id: "tab-1" },
      },
    },
    {
      ReleaseRoomEnvironmentInput: {
        session_id: "session-1",
        target: { kind: "browser_tab", id: "tab-1" },
      },
    },
  ])
  assert.match(notices[0] ?? "", /^Room takeover granted\nRoom environment environment-1/)
  assert.match(notices[1] ?? "", /^Room takeover requires cancellation: action-7\nRoom environment environment-1/)
  assert.match(notices[2] ?? "", /^Room input released\nRoom environment environment-1/)
})

test("/room input commands reject malformed targets without reaching the kernel", async () => {
  const requests: unknown[] = []
  const flashes: string[] = []
  const command = parseSlashCommand("/room takeover tab")
  const cancel = parseSlashCommand("/room cancel")
  assert.equal(command?.kind, "room")
  assert.equal(cancel?.kind, "room")

  await handleRoomSlashCommand({
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return {} as TResponse
    },
    appendNotice: () => undefined,
    flashFooter: (message) => flashes.push(message),
  }, command)
  await handleRoomSlashCommand({
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return {} as TResponse
    },
    appendNotice: () => undefined,
    flashFooter: (message) => flashes.push(message),
  }, cancel)

  assert.deepEqual(requests, [])
  assert.deepEqual(flashes, [
    "usage: /room takeover|release [desktop|tab TAB_ID]",
    "usage: /room cancel ACTION_ID",
  ])
})

test("/room cancel uses the authenticated action-cancellation request", async () => {
  const requests: unknown[] = []
  const notices: string[] = []
  const command = parseSlashCommand("/room cancel action-7")
  assert.equal(command?.kind, "room")

  await handleRoomSlashCommand({
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return {
        RoomEnvironmentActionCancellationUpdated: {
          outcome: { state: "cancelled" },
          environment: roomEnvironment(),
        },
      } as TResponse
    },
    appendNotice: (notice) => notices.push(notice),
    flashFooter: () => undefined,
  }, command)

  assert.deepEqual(requests, [{
    CancelRoomEnvironmentAction: {
      session_id: "session-1",
      action_id: "action-7",
    },
  }])
  assert.match(notices[0] ?? "", /^Room action action-7 cancelled\nRoom environment environment-1/)
})

test("/room cancel renders every authoritative cancellation outcome", async () => {
  const outcomes = [
    [{ state: "cancellation_requested" }, "cancellation requested"],
    [{ state: "already_terminal", action_state: "failed" }, "already failed"],
  ] as const

  for (const [outcome, expected] of outcomes) {
    const notices: string[] = []
    const command = parseSlashCommand("/room cancel action-7")
    assert.equal(command?.kind, "room")

    await handleRoomSlashCommand({
      isAttached: () => true,
      sessionId: () => "session-1",
      send: async <TResponse>() => ({
        RoomEnvironmentActionCancellationUpdated: {
          outcome,
          environment: roomEnvironment(),
        },
      }) as TResponse,
      appendNotice: (notice) => notices.push(notice),
      flashFooter: () => undefined,
    }, command)

    assert.match(
      notices[0] ?? "",
      new RegExp(`^Room action action-7 ${expected}\\nRoom environment environment-1`),
    )
  }
})

function roomEnvironment(): RoomEnvironmentSnapshot {
  return {
    session_id: "session-1",
    environment_id: "environment-1",
    runtime_generation: 2,
    lifecycle: "ready",
    health: [
      { component: "browser_controller", state: "ready", diagnostic_code: null },
      { component: "browser", state: "ready", diagnostic_code: null },
      { component: "desktop", state: "ready", diagnostic_code: null },
      { component: "streamer", state: "ready", diagnostic_code: null },
    ],
    viewport: {
      css_width: 1280,
      css_height: 720,
      device_scale_factor: 1,
      desktop_pixel_width: 1280,
      desktop_pixel_height: 720,
      revision: 3,
      last_actor_id: "agent:agent-1",
    },
    actors: [
      {
        actor_id: "agent:agent-1",
        kind: "agent",
        display_label: "Mara",
        presence: "present",
        presentation_color: "blue",
      },
      {
        actor_id: "user:miguel",
        kind: "human",
        display_label: "Miguel",
        presence: "present",
        presentation_color: "green",
      },
    ],
    pointers: [],
    tabs: [{
      tab_id: "tab-1",
      url: "https://example.test/docs",
      title: "Docs",
      document_revision: 4,
      focused: true,
    }],
    focused_tab_id: "tab-1",
    actions: [],
    input_ownership: [{ target: { kind: "desktop" }, actor_id: "agent:agent-1" }],
    pending_input_takeovers: [],
    event_cursor: 4,
  }
}
