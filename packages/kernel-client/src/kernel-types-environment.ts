export type RoomEnvironmentLifecycle =
  | "stopped"
  | "starting"
  | "ready"
  | "degraded"
  | "saving"
  | "restoring"
  | "stopping"
  | "failed"

export type RoomEnvironmentComponent = "browser_controller" | "browser" | "desktop" | "streamer"

export type RoomEnvironmentComponentHealthState = "starting" | "ready" | "degraded" | "unavailable"

export type RoomEnvironmentComponentHealth = {
  component: RoomEnvironmentComponent
  state: RoomEnvironmentComponentHealthState
  diagnostic_code: string | null
}

export type RoomEnvironmentViewport = {
  css_width: number
  css_height: number
  device_scale_factor: number
  desktop_pixel_width: number
  desktop_pixel_height: number
  revision: number
  last_actor_id: string | null
}

export type RoomEnvironmentActor = {
  actor_id: string
  kind: "human" | "agent"
  display_label: string
  presence: "present" | "away" | "disconnected"
}

export type RoomEnvironmentTab = {
  tab_id: string
  url: string
  title: string
  document_revision: number
  focused: boolean
}

export type RoomEnvironmentInputTarget =
  | { kind: "desktop" }
  | { kind: "browser_tab"; id: string }

export type RoomEnvironmentAction = {
  action_id: string
  sequence: number
  idempotency_key: string | null
  actor_id: string
  runtime_generation: number
  mode: "browser" | "computer"
  kind: string
  targets: RoomEnvironmentInputTarget[]
  state: "queued" | "running" | "completed" | "failed" | "cancelled"
  cancellation_requested: boolean
}

export type RoomEnvironmentInputOwnership = {
  target: RoomEnvironmentInputTarget
  actor_id: string
}

export type RoomEnvironmentPendingInputTakeover = {
  target: RoomEnvironmentInputTarget
  human_actor_id: string
  blocking_action_ids: string[]
}

export type RoomEnvironmentTakeoverOutcome =
  | { state: "granted" }
  | { state: "cancellation_required"; action_ids: string[] }

export type RoomEnvironmentActionCancellationOutcome =
  | { state: "cancelled" }
  | { state: "cancellation_requested" }
  | { state: "already_terminal"; action_state: RoomEnvironmentAction["state"] }

export type RoomEnvironmentSnapshot = {
  session_id: string
  environment_id: string
  runtime_generation: number
  lifecycle: RoomEnvironmentLifecycle
  health: RoomEnvironmentComponentHealth[]
  viewport: RoomEnvironmentViewport
  actors: RoomEnvironmentActor[]
  tabs: RoomEnvironmentTab[]
  focused_tab_id: string | null
  actions: RoomEnvironmentAction[]
  input_ownership: RoomEnvironmentInputOwnership[]
  pending_input_takeovers: RoomEnvironmentPendingInputTakeover[]
  event_cursor: number
}

export type RoomEnvironmentEventKind =
  | { LifecycleChanged: { lifecycle: RoomEnvironmentLifecycle } }
  | "RuntimeInvalidated"
  | "HealthChanged"
  | "TabsChanged"
  | { ViewportChanged: { revision: number } }
  | "ActorsChanged"
  | "InputOwnershipChanged"
  | {
      ActionChanged: {
        action_id: string
        state: RoomEnvironmentAction["state"]
        cancellation_requested: boolean
      }
    }

export type RoomEnvironmentEvent = {
  event_id: number
  environment_id: string
  runtime_generation: number
  kind: RoomEnvironmentEventKind
}

export type RoomEnvironmentReplay =
  | { Events: { events: RoomEnvironmentEvent[]; next_cursor: number } }
  | { SnapshotRequired: { snapshot: RoomEnvironmentSnapshot } }

export type RoomEnvironmentStateResponse = {
  RoomEnvironmentState: {
    environment: RoomEnvironmentSnapshot
  }
}

export type RoomEnvironmentEventsResponse = {
  RoomEnvironmentEvents: {
    replay: RoomEnvironmentReplay
  }
}

export type RoomEnvironmentUpdatedResponse = {
  RoomEnvironmentUpdated: {
    environment: RoomEnvironmentSnapshot
  }
}

export type RoomEnvironmentTakeoverUpdatedResponse = {
  RoomEnvironmentTakeoverUpdated: {
    outcome: RoomEnvironmentTakeoverOutcome
    environment: RoomEnvironmentSnapshot
  }
}

export type RoomEnvironmentInputReleasedResponse = {
  RoomEnvironmentInputReleased: {
    environment: RoomEnvironmentSnapshot
  }
}

export type RoomEnvironmentActionCancellationUpdatedResponse = {
  RoomEnvironmentActionCancellationUpdated: {
    outcome: RoomEnvironmentActionCancellationOutcome
    environment: RoomEnvironmentSnapshot
  }
}
