export function getRoomEnvironmentStateRequest(sessionId: string) {
  return {
    GetRoomEnvironmentState: {
      session_id: sessionId,
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
