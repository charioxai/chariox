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
  presentation_color: "blue" | "cyan" | "green" | "amber" | "orange" | "rose" | "violet" | "slate"
}

export type RoomEnvironmentPointer = {
  actor_id: string
  x: number
  y: number
  viewport_revision: number
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

export type RoomEnvironmentActionArguments =
  | {
      kind: "pointer_click"
      x: number
      y: number
      button: "left" | "middle" | "right"
      click_count: 1 | 2
      viewport_revision: number
    }
  | {
      kind: "clipboard_write"
      utf8_byte_count: number
      character_count: number
    }

export type RoomEnvironmentAction = {
  action_id: string
  sequence: number
  idempotency_key: string | null
  actor_id: string
  runtime_generation: number
  mode: "browser" | "computer"
  kind: string
  arguments?: RoomEnvironmentActionArguments
  targets: RoomEnvironmentInputTarget[]
  state: "queued" | "running" | "completed" | "failed" | "cancelled"
  cancellation_requested: boolean
  submitted_at_ms: number
  started_at_ms: number | null
  finished_at_ms: number | null
  outcome: RoomEnvironmentActionOutcome | null
}

export type RoomEnvironmentActionOutcome =
  | { status: "completed" }
  | { status: "failed"; code: "controller_failure" | "process_lost" }
  | {
      status: "cancelled"
      reason: "requested" | "human_takeover" | "controller_cancellation"
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
  pointers: RoomEnvironmentPointer[]
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
  | "PointersChanged"
  | "InputOwnershipChanged"
  | {
      ActionChanged: {
        action_id: string
        state: RoomEnvironmentAction["state"]
        cancellation_requested: boolean
        submitted_at_ms: number
        started_at_ms: number | null
        finished_at_ms: number | null
        outcome: RoomEnvironmentActionOutcome | null
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

export type RoomEnvironmentSliceBinding = {
  session_id: string
  slice_id: string
  owner_kernel_id: string
  worker_kernel_ref: string
}

export type RoomEnvironmentSliceResponse = {
  RoomEnvironmentSlice: { binding: RoomEnvironmentSliceBinding | null }
}

export type RoomEnvironmentScreenshotArtifact = {
  artifact_id: string
  sha256: string
  size_bytes: number
  media_type: "image/png"
  display_name: string
}

export type RoomEnvironmentScreenshotCapturedResponse = {
  RoomEnvironmentScreenshotCaptured: {
    artifact: RoomEnvironmentScreenshotArtifact
  }
}

export type RoomEnvironmentScreenshotChunk = {
  artifact_id: string
  offset: number
  data_base64: string
  eof: boolean
}

export type RoomEnvironmentScreenshotChunkResponse = {
  RoomEnvironmentScreenshotChunk: {
    chunk: RoomEnvironmentScreenshotChunk
  }
}

export type RoomEnvironmentEventsResponse = {
  RoomEnvironmentEvents: {
    replay: RoomEnvironmentReplay
  }
}

export type RoomEnvironmentActionHistoryResponse = {
  RoomEnvironmentActionHistoryListed: {
    page: {
      actions: RoomEnvironmentAction[]
      next_before_sequence: number | null
    }
  }
}

export type RoomEnvironmentUpdatedResponse = {
  RoomEnvironmentUpdated: {
    environment: RoomEnvironmentSnapshot
  }
}

export type RoomEnvironmentActionSubmittedResponse = {
  RoomEnvironmentActionSubmitted: {
    action_id: string
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

export type RoomEnvironmentClipboardReadResponse = {
  RoomEnvironmentClipboardRead: {
    content: string
  }
}

export type RoomEnvironmentActionCancellationUpdatedResponse = {
  RoomEnvironmentActionCancellationUpdated: {
    outcome: RoomEnvironmentActionCancellationOutcome
    environment: RoomEnvironmentSnapshot
  }
}
