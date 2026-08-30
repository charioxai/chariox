export function getRoomEnvironmentStateRequest(sessionId: string) {
  return {
    GetRoomEnvironmentState: {
      session_id: sessionId,
    },
  }
}

export function getRoomEnvironmentEventsRequest(sessionId: string, cursor: number) {
  return {
    GetRoomEnvironmentEvents: {
      session_id: sessionId,
      cursor,
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
