import {
  getRoomEnvironmentEventsRequest,
  getRoomEnvironmentStateRequest,
} from "@chariox/kernel-client/ipc-requests"
import type {
  RoomEnvironmentAction,
  RoomEnvironmentActionOutcome,
  RoomEnvironmentEvent,
  RoomEnvironmentEventsResponse,
  RoomEnvironmentSnapshot,
  RoomEnvironmentStateResponse,
} from "@chariox/kernel-client/kernel-types"

export type RoomEnvironmentActivityControllerDeps = {
  readonly isAttached: () => boolean
  readonly sessionId: () => string
  readonly nowMs: () => number
  readonly send: <T>(request: unknown) => Promise<T>
  readonly appendNotice: (message: string) => void
  readonly recordDaemonActivity: (kind: string) => void
}

export type RoomEnvironmentActivityController = {
  synchronize(): Promise<boolean>
  reset(): void
}

const missingEnvironmentProbeIntervalMs = 5_000

export function createRoomEnvironmentActivityController(
  deps: RoomEnvironmentActivityControllerDeps,
): RoomEnvironmentActivityController {
  let selectedSessionId: string | null = null
  let selectionRevision = 0
  let environment: RoomEnvironmentSnapshot | null = null
  let replayCursor = 0
  let nextMissingEnvironmentProbeAtMs = 0
  let synchronization: Promise<boolean> | null = null

  const reset = () => {
    selectedSessionId = null
    selectionRevision += 1
    environment = null
    replayCursor = 0
    nextMissingEnvironmentProbeAtMs = 0
    synchronization = null
  }

  const selectCurrentSession = () => {
    const nextSessionId = deps.isAttached() ? deps.sessionId().trim() || null : null
    if (nextSessionId === selectedSessionId) return
    selectedSessionId = nextSessionId
    selectionRevision += 1
    environment = null
    replayCursor = 0
    nextMissingEnvironmentProbeAtMs = 0
    synchronization = null
  }

  const synchronize = (): Promise<boolean> => {
    selectCurrentSession()
    const sessionId = selectedSessionId
    if (!sessionId) return Promise.resolve(false)
    if (!environment && deps.nowMs() < nextMissingEnvironmentProbeAtMs) {
      return Promise.resolve(false)
    }
    if (synchronization) return synchronization

    const revision = selectionRevision
    const pending = performSynchronization(sessionId, revision)
      .catch((error) => {
        if (!selectionMatches(sessionId, revision)) return false
        if (!isMissingEnvironmentError(error)) throw error
        environment = null
        replayCursor = 0
        nextMissingEnvironmentProbeAtMs = deps.nowMs() + missingEnvironmentProbeIntervalMs
        return false
      })
      .finally(() => {
        if (synchronization === pending) synchronization = null
      })
    synchronization = pending
    return pending
  }

  const selectionMatches = (sessionId: string, revision: number) => (
    selectedSessionId === sessionId && selectionRevision === revision
  )

  const performSynchronization = async (
    sessionId: string,
    revision: number,
  ): Promise<boolean> => {
    if (!environment) {
      const response = await deps.send<RoomEnvironmentStateResponse>(
        getRoomEnvironmentStateRequest(sessionId),
      )
      const nextEnvironment = environmentFromStateResponse(response, sessionId)
      if (!selectionMatches(sessionId, revision)) return false
      environment = nextEnvironment
      replayCursor = nextEnvironment.event_cursor
      nextMissingEnvironmentProbeAtMs = 0
      deps.appendNotice(roomEnvironmentSummary("Room screen", nextEnvironment))
      deps.recordDaemonActivity("room_environment_state")
      return true
    }

    const response = await deps.send<RoomEnvironmentEventsResponse>(
      getRoomEnvironmentEventsRequest(sessionId, replayCursor),
    )
    const replay = replayFromEventsResponse(response)
    if ("SnapshotRequired" in replay) {
      const nextEnvironment = requireEnvironmentSnapshot(
        replay.SnapshotRequired.snapshot,
        sessionId,
      )
      assertCursorDoesNotMoveBackwards(replayCursor, nextEnvironment.event_cursor)
      if (!selectionMatches(sessionId, revision)) return false
      environment = nextEnvironment
      replayCursor = nextEnvironment.event_cursor
      deps.appendNotice(roomEnvironmentSummary("Room activity resynchronized", nextEnvironment))
      deps.recordDaemonActivity("room_environment_events")
      return true
    }

    const { events, next_cursor: nextCursor } = replay.Events
    requireReplayCursor(nextCursor)
    assertCursorDoesNotMoveBackwards(replayCursor, nextCursor)
    validateEvents(events, environment, replayCursor, nextCursor)
    if (!events.length) {
      if (!selectionMatches(sessionId, revision)) return false
      replayCursor = nextCursor
      return false
    }

    const stateResponse = await deps.send<RoomEnvironmentStateResponse>(
      getRoomEnvironmentStateRequest(sessionId),
    )
    const nextEnvironment = environmentFromStateResponse(stateResponse, sessionId)
    assertCursorDoesNotMoveBackwards(nextCursor, nextEnvironment.event_cursor)
    if (!selectionMatches(sessionId, revision)) return false
    environment = nextEnvironment
    replayCursor = nextEnvironment.event_cursor
    const notices = roomEnvironmentEventNotices(events, nextEnvironment)
    for (const notice of notices) deps.appendNotice(notice)
    deps.recordDaemonActivity("room_environment_events")
    return notices.length > 0
  }

  return { synchronize, reset }
}

function environmentFromStateResponse(
  response: RoomEnvironmentStateResponse,
  expectedSessionId: string,
): RoomEnvironmentSnapshot {
  if (!response || typeof response !== "object" || !("RoomEnvironmentState" in response)) {
    throw new Error("kernel did not return Room Environment state")
  }
  return requireEnvironmentSnapshot(
    response.RoomEnvironmentState.environment,
    expectedSessionId,
  )
}

function replayFromEventsResponse(response: RoomEnvironmentEventsResponse) {
  if (!response || typeof response !== "object" || !("RoomEnvironmentEvents" in response)) {
    throw new Error("kernel did not return Room Environment events")
  }
  const replay = response.RoomEnvironmentEvents.replay
  if (!replay || typeof replay !== "object") {
    throw new Error("Room Environment replay is malformed")
  }
  return replay
}

function requireEnvironmentSnapshot(
  snapshot: RoomEnvironmentSnapshot,
  expectedSessionId: string,
): RoomEnvironmentSnapshot {
  if (!snapshot || typeof snapshot !== "object") {
    throw new Error("Room Environment snapshot is malformed")
  }
  if (snapshot.session_id !== expectedSessionId) {
    throw new Error(
      `kernel returned Room Environment state for ${snapshot.session_id}, expected ${expectedSessionId}`,
    )
  }
  requireReplayCursor(snapshot.event_cursor)
  if (!Array.isArray(snapshot.actors) || !Array.isArray(snapshot.tabs) || !Array.isArray(snapshot.actions)) {
    throw new Error("Room Environment snapshot collections are malformed")
  }
  return snapshot
}

function validateEvents(
  events: RoomEnvironmentEvent[],
  environment: RoomEnvironmentSnapshot,
  previousCursor: number,
  nextCursor: number,
): void {
  if (!Array.isArray(events) || events.length > 4_096) {
    throw new Error("Room Environment event replay is malformed")
  }
  let lastEventId = previousCursor
  for (const event of events) {
    if (
      !event
      || typeof event !== "object"
      || !Number.isSafeInteger(event.event_id)
      || event.event_id <= lastEventId
      || event.event_id > nextCursor
      || event.environment_id !== environment.environment_id
    ) {
      throw new Error("Room Environment event replay is malformed")
    }
    lastEventId = event.event_id
  }
}

function roomEnvironmentSummary(prefix: string, environment: RoomEnvironmentSnapshot): string {
  const tab = focusedTabLabel(environment)
  const actors = environment.actors
    .filter((actor) => actor.presence !== "disconnected")
    .map((actor) => actor.display_label)
    .join(", ") || "none"
  const inputOwner = desktopInputOwnerLabel(environment)
  return `${prefix}: ${environment.lifecycle} · tab ${tab} · actors ${actors} · input ${inputOwner}`
}

function roomEnvironmentEventNotices(
  events: RoomEnvironmentEvent[],
  environment: RoomEnvironmentSnapshot,
): string[] {
  const notices: string[] = []
  const emittedKinds = new Set<string>()
  const emittedActions = new Set<string>()
  for (const event of events) {
    const kind = event.kind
    if (kind === "PointersChanged") continue
    if (typeof kind === "object" && "ActionChanged" in kind) {
      const actionId = kind.ActionChanged.action_id
      if (emittedActions.has(actionId)) continue
      emittedActions.add(actionId)
      const action = environment.actions.find((candidate) => candidate.action_id === actionId)
      notices.push(formatActionNotice(action, kind.ActionChanged, environment))
      continue
    }
    const key = typeof kind === "string" ? kind : Object.keys(kind)[0] ?? "unknown"
    if (emittedKinds.has(key)) continue
    emittedKinds.add(key)
    if (kind === "ActorsChanged") {
      const actors = environment.actors
        .filter((actor) => actor.presence !== "disconnected")
        .map((actor) => `${actor.display_label} (${actor.presence})`)
      notices.push(`Room actors: ${actors.join(", ") || "none"}`)
    } else if (kind === "TabsChanged") {
      notices.push(`Room tab: ${focusedTabLabel(environment)}`)
    } else if (kind === "InputOwnershipChanged") {
      const owner = environment.input_ownership.find((item) => item.target.kind === "desktop")
      notices.push(owner
        ? `Room input: ${desktopInputOwnerLabel(environment)} controls desktop`
        : "Room input: available")
    } else if (kind === "HealthChanged") {
      notices.push(`Room health: ${formatHealth(environment)}`)
    } else if (kind === "RuntimeInvalidated") {
      notices.push(`Room runtime restarted: generation ${environment.runtime_generation}`)
    } else if (typeof kind === "object" && "LifecycleChanged" in kind) {
      notices.push(`Room environment: ${kind.LifecycleChanged.lifecycle}`)
    } else if (typeof kind === "object" && "ViewportChanged" in kind) {
      notices.push(
        `Room viewport: ${environment.viewport.desktop_pixel_width}×${environment.viewport.desktop_pixel_height}`,
      )
    }
  }
  return notices
}

function formatActionNotice(
  action: RoomEnvironmentAction | undefined,
  changed: Extract<RoomEnvironmentEvent["kind"], { ActionChanged: unknown }>["ActionChanged"],
  environment: RoomEnvironmentSnapshot,
): string {
  if (!action) return `Room action: ${changed.action_id} · ${formatActionState(changed.state, changed.outcome)}`
  const actor = environment.actors.find((candidate) => candidate.actor_id === action.actor_id)
  return [
    `Room action #${action.sequence}:`,
    actor?.display_label ?? actorLabel(action.actor_id),
    "·",
    action.mode,
    action.kind,
    "·",
    formatActionState(changed.state, changed.outcome ?? action.outcome),
  ].join(" ")
}

function actorLabel(actorId: string): string {
  const separator = actorId.indexOf(":")
  const raw = separator >= 0 ? actorId.slice(separator + 1) : actorId
  return raw ? `${raw[0]?.toUpperCase()}${raw.slice(1)}` : actorId
}

function formatActionState(
  state: RoomEnvironmentAction["state"],
  outcome: RoomEnvironmentActionOutcome | null,
): string {
  if (!outcome) return state
  if (outcome.status === "failed") return `failed (${outcome.code})`
  if (outcome.status === "cancelled") return `cancelled (${outcome.reason})`
  return outcome.status
}

function focusedTabLabel(environment: RoomEnvironmentSnapshot): string {
  const focused = environment.tabs.find((tab) => tab.tab_id === environment.focused_tab_id)
    ?? environment.tabs.find((tab) => tab.focused)
  if (!focused) return "none"
  const title = focused.title.trim()
  return title && title !== focused.url ? `${title} — ${focused.url}` : focused.url
}

function desktopInputOwnerLabel(environment: RoomEnvironmentSnapshot): string {
  const owner = environment.input_ownership.find((item) => item.target.kind === "desktop")
  if (!owner) return "available"
  return environment.actors.find((actor) => actor.actor_id === owner.actor_id)?.display_label
    ?? actorLabel(owner.actor_id)
}

function formatHealth(environment: RoomEnvironmentSnapshot): string {
  const unhealthy = environment.health.filter((health) => health.state !== "ready")
  if (!unhealthy.length) return "ready"
  return unhealthy
    .map((health) => `${health.component} ${health.state}${health.diagnostic_code ? ` (${health.diagnostic_code})` : ""}`)
    .join(", ")
}

function requireReplayCursor(cursor: number): void {
  if (!Number.isSafeInteger(cursor) || cursor < 0) {
    throw new Error("Room Environment replay cursor is invalid")
  }
}

function assertCursorDoesNotMoveBackwards(previousCursor: number, nextCursor: number): void {
  if (nextCursor < previousCursor) {
    throw new Error(
      `Room Environment replay cursor moved backwards from ${previousCursor} to ${nextCursor}`,
    )
  }
}

function isMissingEnvironmentError(error: unknown): boolean {
  if (!error || typeof error !== "object") return false
  const record = error as { readonly code?: unknown; readonly message?: unknown }
  return record.code === "environment_not_found"
    || (typeof record.message === "string" && /environment_not_found/i.test(record.message))
}
