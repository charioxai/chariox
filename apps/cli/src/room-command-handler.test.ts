import assert from "node:assert/strict"
import test from "node:test"

import type { RoomEnvironmentAction, RoomEnvironmentSnapshot } from "@chariox/kernel-client/kernel-types"

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

test("/room actions renders bounded browser and computer history with a continuation cursor", async () => {
  const requests: unknown[] = []
  const notices: string[] = []
  const command = parseSlashCommand("/room actions")
  assert.equal(command?.kind, "room")

  await handleRoomSlashCommand({
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return {
        RoomEnvironmentActionHistoryListed: {
          page: {
            actions: [
              roomAction({
                action_id: "action-42",
                sequence: 42,
                mode: "browser",
                kind: "navigate",
                targets: [{ kind: "browser_tab", id: "tab-1" }],
                submitted_at_ms: 1_788_300_000_042,
              }),
              roomAction({
                action_id: "action-41",
                sequence: 41,
                actor_id: "user:miguel",
                state: "failed",
                outcome: { status: "failed", code: "controller_failure" },
                submitted_at_ms: 1_788_300_000_041,
              }),
            ],
            next_before_sequence: 41,
          },
        },
      } as TResponse
    },
    appendNotice: (notice) => notices.push(notice),
    flashFooter: () => undefined,
  }, command)

  assert.deepEqual(requests, [{
    ListRoomEnvironmentActionHistory: {
      session_id: "session-1",
      before_sequence: null,
      limit: 20,
    },
  }])
  assert.deepEqual(notices, [[
    "Room actions (2)",
    "#42 action-42 actor=agent:agent-1 browser:navigate target=tab:tab-1 state=completed submitted_at_ms=1788300000042",
    "#41 action-41 actor=user:miguel computer:pointer_click target=desktop state=failed(controller_failure) submitted_at_ms=1788300000041",
    "next_before=41",
  ].join("\n")])
})

test("/room actions accepts explicit pagination and rejects unsafe bounds", async () => {
  const requests: unknown[] = []
  const notices: string[] = []
  const flashes: string[] = []
  const deps = {
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return {
        RoomEnvironmentActionHistoryListed: {
          page: { actions: [], next_before_sequence: null },
        },
      } as TResponse
    },
    appendNotice: (notice: string) => notices.push(notice),
    flashFooter: (message: string) => flashes.push(message),
  }
  const valid = parseSlashCommand("/room actions 5 42")
  const invalid = [
    "/room actions 0",
    "/room actions 101",
    "/room actions five",
    "/room actions 5 0",
    "/room actions 5 42 extra",
  ].map(parseSlashCommand)
  assert.equal(valid?.kind, "room")
  assert.ok(invalid.every((command) => command?.kind === "room"))

  await handleRoomSlashCommand(deps, valid)
  for (const command of invalid) {
    assert.equal(command?.kind, "room")
    await handleRoomSlashCommand(deps, command)
  }

  assert.deepEqual(requests, [{
    ListRoomEnvironmentActionHistory: {
      session_id: "session-1",
      before_sequence: 42,
      limit: 5,
    },
  }])
  assert.deepEqual(notices, ["Room actions: none"])
  assert.deepEqual(flashes, Array(5).fill("usage: /room actions [LIMIT] [BEFORE_SEQUENCE]"))
})

test("/room browser submits focused-tab history through authenticated Room authority", async () => {
  const requests: unknown[] = []
  const notices: string[] = []
  const command = parseSlashCommand("/room browser back")
  assert.equal(command?.kind, "room")

  await handleRoomSlashCommand({
    isAttached: () => true,
    sessionId: () => "session-1",
    createIdempotencyKey: () => "tui-history-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      if (Object.prototype.hasOwnProperty.call(request, "GetRoomEnvironmentState")) {
        return { RoomEnvironmentState: { environment: roomEnvironment() } } as TResponse
      }
      return {
        RoomEnvironmentActionSubmitted: {
          action_id: "action-history-1",
          environment: {
            ...roomEnvironment(),
            tabs: [{
              ...roomEnvironment().tabs[0],
              url: "https://example.test/previous",
              document_revision: 5,
            }],
            event_cursor: 5,
          },
        },
      } as TResponse
    },
    appendNotice: (notice) => notices.push(notice),
    flashFooter: () => undefined,
  }, command)

  assert.deepEqual(requests, [
    { GetRoomEnvironmentState: { session_id: "session-1" } },
    {
      SubmitRoomEnvironmentBrowserAction: {
        session_id: "session-1",
        runtime_generation: 2,
        idempotency_key: "tui-history-1",
        action: { kind: "history", tab_id: "tab-1", action: "back" },
      },
    },
  ])
  assert.match(notices[0] ?? "", /^Room browser back submitted as action-history-1\nRoom environment environment-1/)
  assert.match(notices[0] ?? "", /tab=tab-1 Docs — https:\/\/example\.test\/previous/)
})

test("/room browser preserves explicit stable tabs for forward and reload", async () => {
  const environment = roomEnvironment()
  const requests: unknown[] = []
  let idempotencyKeyIndex = 0
  const idempotencyKeys = ["forward-1", "reload-1"]
  const tabTwo = {
    ...environment.tabs[0],
    tab_id: "tab-2",
    title: "Second",
    url: "https://example.test/second",
    focused: false,
  }
  const commands = [
    parseSlashCommand("/room browser forward tab-2"),
    parseSlashCommand("/room browser reload tab-1"),
  ]
  assert.equal(commands.every((command) => command?.kind === "room"), true)

  for (const command of commands) {
    if (!command || command.kind !== "room") throw new Error("Room browser command should parse")
    await handleRoomSlashCommand({
      isAttached: () => true,
      sessionId: () => "session-1",
      createIdempotencyKey: () => idempotencyKeys[idempotencyKeyIndex++] ?? "unexpected-key",
      send: async <TResponse>(request: unknown) => {
        requests.push(request)
        if (Object.prototype.hasOwnProperty.call(request, "GetRoomEnvironmentState")) {
          return {
            RoomEnvironmentState: {
              environment: { ...environment, tabs: [...environment.tabs, tabTwo] },
            },
          } as TResponse
        }
        return {
          RoomEnvironmentActionSubmitted: {
            action_id: `action-${requests.length}`,
            environment,
          },
        } as TResponse
      },
      appendNotice: () => undefined,
      flashFooter: () => undefined,
    }, command)
  }

  assert.deepEqual(requests.filter((request) => (
    Object.prototype.hasOwnProperty.call(request, "SubmitRoomEnvironmentBrowserAction")
  )), [
    {
      SubmitRoomEnvironmentBrowserAction: {
        session_id: "session-1",
        runtime_generation: 2,
        idempotency_key: "forward-1",
        action: { kind: "history", tab_id: "tab-2", action: "forward" },
      },
    },
    {
      SubmitRoomEnvironmentBrowserAction: {
        session_id: "session-1",
        runtime_generation: 2,
        idempotency_key: "reload-1",
        action: { kind: "history", tab_id: "tab-1", action: "reload" },
      },
    },
  ])
})

test("/room browser validates its action and stable tab before mutation", async () => {
  const requests: unknown[] = []
  const flashes: string[] = []
  const deps = {
    isAttached: () => true,
    sessionId: () => "session-1",
    createIdempotencyKey: () => "must-not-be-used",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return { RoomEnvironmentState: { environment: roomEnvironment() } } as TResponse
    },
    appendNotice: () => undefined,
    flashFooter: (message: string) => flashes.push(message),
  }
  const invalid = parseSlashCommand("/room browser close")
  const unknownTab = parseSlashCommand("/room browser reload missing-tab")
  assert.equal(invalid?.kind, "room")
  assert.equal(unknownTab?.kind, "room")

  await handleRoomSlashCommand(deps, invalid)
  await handleRoomSlashCommand(deps, unknownTab)

  assert.deepEqual(requests, [{ GetRoomEnvironmentState: { session_id: "session-1" } }])
  assert.deepEqual(flashes, [
    "usage: /room browser back|forward|reload [TAB_ID]",
    "Room browser tab missing-tab is not present; run /room status and retry with a current tab ID",
  ])
})

test("/room browser requires an authoritative focused tab when no tab ID is supplied", async () => {
  const requests: unknown[] = []
  const flashes: string[] = []
  const environment = roomEnvironment()
  const command = parseSlashCommand("/room browser reload")
  assert.equal(command?.kind, "room")

  await handleRoomSlashCommand({
    isAttached: () => true,
    sessionId: () => "session-1",
    createIdempotencyKey: () => "must-not-be-used",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return {
        RoomEnvironmentState: {
          environment: {
            ...environment,
            focused_tab_id: null,
            tabs: environment.tabs.map((tab) => ({ ...tab, focused: false })),
          },
        },
      } as TResponse
    },
    appendNotice: () => undefined,
    flashFooter: (message) => flashes.push(message),
  }, command)

  assert.deepEqual(requests, [{ GetRoomEnvironmentState: { session_id: "session-1" } }])
  assert.deepEqual(flashes, [
    "Room browser has no focused tab; run /room status and retry with an explicit tab ID",
  ])
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
    "usage: /room status|actions [LIMIT] [BEFORE_SEQUENCE]|start [WIDTHxHEIGHT] [SCALE]|stop|retry|reconnect|view|screenshot|browser back|forward|reload [TAB_ID]|takeover|release [desktop|tab TAB_ID]|cancel ACTION_ID|save restart|shutdown",
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

test("/room save uses the bound Environment slice and preserves the requested lifecycle", async () => {
  const requests: unknown[] = []
  const notices: string[] = []
  const deps = {
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      if (Object.prototype.hasOwnProperty.call(request, "GetRoomEnvironmentSlice")) {
        return {
          RoomEnvironmentSlice: {
            binding: {
              session_id: "session-1",
              slice_id: "slice-1",
              owner_kernel_id: "kernel-home",
              worker_kernel_ref: "kernel-worker",
            },
          },
        } as TResponse
      }
      return {
        SliceStateSaved: {
          slice: {
            id: "slice-1",
            name: "desktop",
          },
          state: {
            id: "state-1",
            image_ref: "chariox/slice-state:state-1",
            home_archive_path: "/var/lib/chariox/states/state-1-home.tar.zst",
          },
        },
      } as TResponse
    },
    appendNotice: (notice: string) => notices.push(notice),
    flashFooter: () => undefined,
  }
  const restart = parseSlashCommand("/room save restart")
  const shutdown = parseSlashCommand("/room save shutdown")
  assert.equal(restart?.kind, "room")
  assert.equal(shutdown?.kind, "room")

  await handleRoomSlashCommand(deps, restart)
  await handleRoomSlashCommand(deps, shutdown)

  assert.deepEqual(requests, [
    { GetRoomEnvironmentSlice: { session_id: "session-1" } },
    {
      SaveSliceState: {
        slice_ref: "slice-1",
        mode: "restart_agents",
        scope: "this_slice",
      },
    },
    { GetRoomEnvironmentSlice: { session_id: "session-1" } },
    {
      SaveSliceState: {
        slice_ref: "slice-1",
        mode: "shutdown",
        scope: "this_slice",
      },
    },
  ])
  assert.deepEqual(notices, [
    "saved slice state desktop (slice-1)\nstate=state-1\nimage=chariox/slice-state:state-1\nhome_archive=/var/lib/chariox/states/state-1-home.tar.zst",
    "saved slice state desktop (slice-1)\nstate=state-1\nimage=chariox/slice-state:state-1\nhome_archive=/var/lib/chariox/states/state-1-home.tar.zst",
  ])
})

test("/room save rejects invalid modes and an Environment without a bound slice", async () => {
  const requests: unknown[] = []
  const flashes: string[] = []
  const deps = {
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return { RoomEnvironmentSlice: { binding: null } } as TResponse
    },
    appendNotice: () => undefined,
    flashFooter: (message: string) => flashes.push(message),
  }
  const invalid = parseSlashCommand("/room save later")
  const unbound = parseSlashCommand("/room save restart")
  assert.equal(invalid?.kind, "room")
  assert.equal(unbound?.kind, "room")

  await handleRoomSlashCommand(deps, invalid)
  await handleRoomSlashCommand(deps, unbound)

  assert.deepEqual(requests, [
    { GetRoomEnvironmentSlice: { session_id: "session-1" } },
  ])
  assert.deepEqual(flashes, [
    "usage: /room save restart|shutdown",
    "Room Environment has no bound slice to save",
  ])
})

test("/room view opens the focused agent's bound Environment in Chariox Cloud", async () => {
  const requests: unknown[] = []
  const viewerTargets: unknown[] = []
  const notices: string[] = []
  const command = parseSlashCommand("/room view")
  assert.equal(command?.kind, "room")

  await handleRoomSlashCommand({
    isAttached: () => true,
    sessionId: () => "session-1",
    focusedAgentId: () => "agent-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return {
        RoomEnvironmentSlice: {
          binding: {
            session_id: "session-1",
            slice_id: "slice-1",
            owner_kernel_id: "kernel-home",
            worker_kernel_ref: "kernel-worker",
          },
        },
      } as TResponse
    },
    openViewer: async (target) => {
      viewerTargets.push(target)
      return {
        url: "https://cloud.test/view?view_target=session-1%3Aagent-1%3Aslice-1",
        opened: true,
      }
    },
    appendNotice: (notice) => notices.push(notice),
    flashFooter: () => undefined,
  }, command)

  assert.deepEqual(requests, [
    { GetRoomEnvironmentSlice: { session_id: "session-1" } },
  ])
  assert.deepEqual(viewerTargets, [{
    sessionId: "session-1",
    agentId: "agent-1",
    sliceId: "slice-1",
  }])
  assert.deepEqual(notices, [
    "Opening Room Environment in Chariox Cloud.\nurl=https://cloud.test/view?view_target=session-1%3Aagent-1%3Aslice-1\nbrowser=opened",
  ])
})

test("/room view reports missing focus, slice, and Cloud configuration", async () => {
  const requests: unknown[] = []
  const flashes: string[] = []
  const command = parseSlashCommand("/room view")
  assert.equal(command?.kind, "room")
  const common = {
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return { RoomEnvironmentSlice: { binding: null } } as TResponse
    },
    appendNotice: () => undefined,
    flashFooter: (message: string) => flashes.push(message),
  }

  await handleRoomSlashCommand({ ...common, focusedAgentId: () => null }, command)
  await handleRoomSlashCommand({ ...common, focusedAgentId: () => "agent-1" }, command)
  await handleRoomSlashCommand({
    ...common,
    focusedAgentId: () => "agent-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return {
        RoomEnvironmentSlice: {
          binding: { session_id: "session-1", slice_id: "slice-1" },
        },
      } as TResponse
    },
  }, command)

  assert.deepEqual(requests, [
    { GetRoomEnvironmentSlice: { session_id: "session-1" } },
    { GetRoomEnvironmentSlice: { session_id: "session-1" } },
  ])
  assert.deepEqual(flashes, [
    "focus an agent before opening the Room Environment",
    "Room Environment has no bound slice to view",
    "Chariox Cloud Web View is not configured; run /cloud link first",
  ])
})

test("/room screenshot saves the Room Environment image on the TUI host", async () => {
  const notices: string[] = []
  let captures = 0
  const command = parseSlashCommand("/room screenshot")
  assert.equal(command?.kind, "room")

  await handleRoomSlashCommand({
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>() => ({} as TResponse),
    captureScreenshot: async () => {
      captures += 1
      return {
        artifact: {
          artifact_id: "artifact-1",
          sha256: "bef57ec7f53a6d40beb640a780a639c83bc29ac8a9816f1fc6c5c6dcd93c4721",
          size_bytes: 6,
          media_type: "image/png",
          display_name: "capture.png",
        },
        path: "/Users/example/Downloads/capture.png",
      }
    },
    appendNotice: (notice) => notices.push(notice),
    flashFooter: () => undefined,
  }, command)

  assert.equal(captures, 1)
  assert.deepEqual(notices, [
    "Room Environment screenshot saved.\npath=/Users/example/Downloads/capture.png\nartifact=artifact-1\nsha256=bef57ec7f53a6d40beb640a780a639c83bc29ac8a9816f1fc6c5c6dcd93c4721",
  ])
})

test("/room reconnect resumes the attached Room event stream without a kernel mutation", async () => {
  const requests: unknown[] = []
  const notices: string[] = []
  let reconnects = 0
  const deps = {
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>(request: unknown) => {
      requests.push(request)
      return {} as TResponse
    },
    reconnectEventStream: async () => {
      reconnects += 1
      return true
    },
    appendNotice: (notice: string) => notices.push(notice),
    flashFooter: () => undefined,
  }
  const reconnect = parseSlashCommand("/room reconnect")
  assert.equal(reconnect?.kind, "room")

  await handleRoomSlashCommand(deps, reconnect)

  assert.equal(reconnects, 1)
  assert.deepEqual(requests, [])
  assert.deepEqual(notices, [
    "Room reconnect requested; events will resume from the last received event.",
  ])
})

test("/room reconnect reports polling transports that have no event stream", async () => {
  const notices: string[] = []
  const flashes: string[] = []
  const command = parseSlashCommand("/room reconnect")
  assert.equal(command?.kind, "room")

  await handleRoomSlashCommand({
    isAttached: () => true,
    sessionId: () => "session-1",
    send: async <TResponse>() => ({} as TResponse),
    reconnectEventStream: async () => false,
    appendNotice: (notice) => notices.push(notice),
    flashFooter: (message) => flashes.push(message),
  }, command)

  assert.deepEqual(notices, [])
  assert.deepEqual(flashes, ["Room reconnect is unavailable on this polling transport"])
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

function roomAction(overrides: Partial<RoomEnvironmentAction> = {}): RoomEnvironmentAction {
  return {
    action_id: "action-1",
    sequence: 1,
    idempotency_key: null,
    actor_id: "agent:agent-1",
    runtime_generation: 2,
    mode: "computer",
    kind: "pointer_click",
    targets: [{ kind: "desktop" }],
    state: "completed",
    cancellation_requested: false,
    submitted_at_ms: 1_788_300_000_001,
    started_at_ms: 1_788_300_000_002,
    finished_at_ms: 1_788_300_000_003,
    outcome: { status: "completed" },
    ...overrides,
  }
}
