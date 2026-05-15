import type {
  CaptureScreenshotResult,
  StoredTransferArtifact,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  captureScreenshotRequest,
  storeTransferredFileRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export async function storeTransferredFile(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
  filePath: string,
  filename: string,
): Promise<StoredTransferArtifact> {
  const response = await client.send<Record<string, unknown>>(
    storeTransferredFileRequest(sessionId, attachmentId, filePath, filename),
  )
  const payload = expectVariant<{ result: StoredTransferArtifact }>(response, "FileTransferred")
  return payload.result
}

export async function captureScreenshot(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string,
): Promise<CaptureScreenshotResult> {
  const response = await client.send<Record<string, unknown>>(captureScreenshotRequest(sessionId, attachmentId))
  const payload = expectVariant<{ result: CaptureScreenshotResult }>(response, "ScreenshotCaptured")
  return payload.result
}
