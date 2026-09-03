export function getRoomEnvironmentStateRequest(sessionId: string) {
  return {
    GetRoomEnvironmentState: {
      session_id: sessionId,
    },
  }
}

export function getRoomEnvironmentSliceRequest(sessionId: string) {
  return { GetRoomEnvironmentSlice: { session_id: sessionId } }
}

export function captureRoomEnvironmentScreenshotRequest(
  sessionId: string,
  attachmentId: string,
) {
  return {
    CaptureRoomEnvironmentScreenshot: {
      session_id: sessionId,
      attachment_id: attachmentId,
    },
  }
}

export function readRoomEnvironmentScreenshotChunkRequest(
  sessionId: string,
  attachmentId: string,
  artifactId: string,
  offset: number,
  maxBytes: number,
) {
  return {
    ReadRoomEnvironmentScreenshotChunk: {
      session_id: sessionId,
      attachment_id: attachmentId,
      artifact_id: artifactId,
      offset,
      max_bytes: maxBytes,
    },
  }
}

export function bindRoomEnvironmentSliceRequest(sessionId: string, sliceRef: string) {
  return { BindRoomEnvironmentSlice: { session_id: sessionId, slice_ref: sliceRef } }
}

export function getRoomEnvironmentEventsRequest(sessionId: string, cursor: number) {
  return {
    GetRoomEnvironmentEvents: {
      session_id: sessionId,
      cursor,
    },
  }
}

export function listRoomEnvironmentActionHistoryRequest(
  sessionId: string,
  beforeSequence: number | null = null,
  limit: number | null = null,
) {
  return {
    ListRoomEnvironmentActionHistory: {
      session_id: sessionId,
      before_sequence: beforeSequence,
      limit,
    },
  }
}

export type RoomEnvironmentViewportRequest = {
  css_width: number
  css_height: number
  device_scale_factor: number
  desktop_pixel_width: number
  desktop_pixel_height: number
}

export function startRoomEnvironmentRequest(
  sessionId: string,
  viewport: RoomEnvironmentViewportRequest,
) {
  return {
    StartRoomEnvironment: {
      session_id: sessionId,
      viewport,
    },
  }
}

export function stopRoomEnvironmentRequest(sessionId: string) {
  return {
    StopRoomEnvironment: {
      session_id: sessionId,
    },
  }
}

export function retryRoomEnvironmentRequest(sessionId: string) {
  return {
    RetryRoomEnvironment: {
      session_id: sessionId,
    },
  }
}

export function updateRoomEnvironmentViewportRequest(
  sessionId: string,
  expectedRevision: number,
  viewport: RoomEnvironmentViewportRequest,
) {
  return {
    UpdateRoomEnvironmentViewport: {
      session_id: sessionId,
      expected_revision: expectedRevision,
      viewport,
    },
  }
}

export type RoomEnvironmentPointerPositionRequest = {
  x: number
  y: number
}

export function updateRoomEnvironmentPointerRequest(
  sessionId: string,
  runtimeGeneration: number,
  viewportRevision: number,
  pointer: RoomEnvironmentPointerPositionRequest | null,
) {
  return {
    UpdateRoomEnvironmentPointer: {
      session_id: sessionId,
      runtime_generation: runtimeGeneration,
      viewport_revision: viewportRevision,
      pointer,
    },
  }
}

export function requestRoomEnvironmentInputTakeoverRequest(
  sessionId: string,
  target: import("./kernel-types-environment.js").RoomEnvironmentInputTarget,
) {
  return {
    RequestRoomEnvironmentInputTakeover: {
      session_id: sessionId,
      target,
    },
  }
}

export function releaseRoomEnvironmentInputRequest(
  sessionId: string,
  target: import("./kernel-types-environment.js").RoomEnvironmentInputTarget,
) {
  return {
    ReleaseRoomEnvironmentInput: {
      session_id: sessionId,
      target,
    },
  }
}

export function cancelRoomEnvironmentActionRequest(sessionId: string, actionId: string) {
  return {
    CancelRoomEnvironmentAction: {
      session_id: sessionId,
      action_id: actionId,
    },
  }
}

export type RoomEnvironmentHumanAction =
  | {
      readonly kind: "keyboard_text"
      readonly text: string
    }
  | {
      readonly kind: "keyboard_key"
      readonly key: string
      readonly repeat: number
    }
  | {
      readonly kind: "pointer_move"
      readonly x: number
      readonly y: number
    }
  | {
      readonly kind: "pointer_drag"
      readonly from_x: number
      readonly from_y: number
      readonly to_x: number
      readonly to_y: number
      readonly button: "left" | "middle" | "right"
    }
  | {
      readonly kind: "pointer_scroll"
      readonly x: number
      readonly y: number
      readonly horizontal_steps: number
      readonly vertical_steps: number
    }
  | {
      readonly kind: "pointer_click"
      readonly x: number
      readonly y: number
      readonly button: "left" | "middle" | "right"
      readonly click_count: 1 | 2
    }

export function submitRoomEnvironmentActionRequest(
  sessionId: string,
  runtimeGeneration: number,
  viewportRevision: number,
  idempotencyKey: string,
  action: RoomEnvironmentHumanAction,
) {
  return {
    SubmitRoomEnvironmentAction: {
      session_id: sessionId,
      runtime_generation: runtimeGeneration,
      viewport_revision: viewportRevision,
      idempotency_key: idempotencyKey,
      action,
    },
  }
}
