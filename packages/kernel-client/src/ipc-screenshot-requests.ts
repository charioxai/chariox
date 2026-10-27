export const visibleRegionCaptureMinimumProtocolVersion = 443
export type ScreenshotSurface =
  | {kind: "room"; session_id: string; attachment_id: string; runtime_generation: number; viewport_revision: number}
  | {kind: "kernel_browser"; tab_id: string; generation: number}
  | {kind: "user_app_view"; view_id: string; generation: number}
export type ScreenshotRegion = {x: number; y: number; width: number; height: number; viewport_width: number; viewport_height: number; frame_width: number; frame_height: number}
export type VisibleRegionCapture = {capture_id: string; surface: ScreenshotSurface; captured_at_ms: number; width: number; height: number; media_type: "image/png"; display_name: string; sha256: string; data_base64: string}
export function captureVisibleRegionRequest(captureId: string, surface: ScreenshotSurface, region: ScreenshotRegion) {
  return {CaptureVisibleRegion: {capture_id: captureId, surface, region}}
}
