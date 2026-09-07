import { createHash } from "node:crypto"
import { mkdir, open, rename, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"

import {
  captureRoomEnvironmentScreenshotRequest,
  readRoomEnvironmentScreenshotChunkRequest,
  roomEnvironmentScreenshotMinimumProtocolVersion,
} from "@chariox/kernel-client/ipc-requests"
import type {
  RoomEnvironmentScreenshotArtifact,
  RoomEnvironmentScreenshotCapturedResponse,
  RoomEnvironmentScreenshotChunkResponse,
} from "@chariox/kernel-client/kernel-types"

import { sendWithProtocolMinimum } from "./protocol-minimum-diagnostic.js"

const SCREENSHOT_CHUNK_BYTES = 128 * 1024
const MAX_SCREENSHOT_BYTES = 64 * 1024 * 1024

export type DownloadRoomEnvironmentScreenshotOptions = {
  sessionId: string
  attachmentId: string
  outputRoot: string
  send: <TResponse>(request: unknown) => Promise<TResponse>
}

export type DownloadedRoomEnvironmentScreenshot = {
  artifact: RoomEnvironmentScreenshotArtifact
  path: string
}

export function defaultRoomScreenshotOutputRoot(): string {
  return path.join(os.homedir(), "Downloads", "Chariox")
}

export async function downloadRoomEnvironmentScreenshot(
  options: DownloadRoomEnvironmentScreenshotOptions,
): Promise<DownloadedRoomEnvironmentScreenshot> {
  const captured = await sendWithProtocolMinimum<RoomEnvironmentScreenshotCapturedResponse>(
    options.send,
    captureRoomEnvironmentScreenshotRequest(options.sessionId, options.attachmentId),
    {
      capability: "Room screenshot capture",
      requestVariant: "CaptureRoomEnvironmentScreenshot",
      minimumProtocolVersion: roomEnvironmentScreenshotMinimumProtocolVersion,
    },
  )
  if (!captured || typeof captured !== "object" || !("RoomEnvironmentScreenshotCaptured" in captured)) {
    throw new Error("Room Environment screenshot response is malformed")
  }
  const artifact = captured.RoomEnvironmentScreenshotCaptured.artifact
  validateArtifact(artifact)
  await mkdir(options.outputRoot, { recursive: true, mode: 0o700 })
  const outputPath = path.join(options.outputRoot, path.basename(artifact.display_name))
  const partialPath = `${outputPath}.partial-${process.pid}`
  const file = await open(partialPath, "w", 0o600)
  const hasher = createHash("sha256")
  let offset = 0
  try {
    while (offset < artifact.size_bytes) {
      const response = await sendWithProtocolMinimum<RoomEnvironmentScreenshotChunkResponse>(
        options.send,
        readRoomEnvironmentScreenshotChunkRequest(
          options.sessionId,
          options.attachmentId,
          artifact.artifact_id,
          offset,
          SCREENSHOT_CHUNK_BYTES,
        ),
        {
          capability: "Room screenshot transfer",
          requestVariant: "ReadRoomEnvironmentScreenshotChunk",
          minimumProtocolVersion: roomEnvironmentScreenshotMinimumProtocolVersion,
        },
      )
      if (!response || typeof response !== "object" || !("RoomEnvironmentScreenshotChunk" in response)) {
        throw new Error("Room Environment screenshot chunk response is malformed")
      }
      const chunk = response.RoomEnvironmentScreenshotChunk.chunk
      if (chunk.artifact_id !== artifact.artifact_id || chunk.offset !== offset) {
        throw new Error("Room Environment screenshot chunk does not match the requested artifact offset")
      }
      const bytes = Buffer.from(chunk.data_base64, "base64")
      if (bytes.length === 0 || bytes.length > SCREENSHOT_CHUNK_BYTES) {
        throw new Error("Room Environment screenshot chunk size is invalid")
      }
      if (offset + bytes.length > artifact.size_bytes) {
        throw new Error("Room Environment screenshot chunk exceeds the declared artifact size")
      }
      await file.write(bytes)
      hasher.update(bytes)
      offset += bytes.length
      if (chunk.eof !== (offset === artifact.size_bytes)) {
        throw new Error("Room Environment screenshot chunk end marker is inconsistent")
      }
    }
    if (hasher.digest("hex") !== artifact.sha256) {
      throw new Error("Room Environment screenshot digest verification failed")
    }
    await file.close()
    await rename(partialPath, outputPath)
    return { artifact, path: outputPath }
  } catch (error) {
    await file.close().catch(() => undefined)
    await rm(partialPath, { force: true }).catch(() => undefined)
    throw error
  }
}

function validateArtifact(artifact: RoomEnvironmentScreenshotArtifact): void {
  if (
    !artifact
    || artifact.media_type !== "image/png"
    || !artifact.artifact_id
    || !/^[a-f0-9]{64}$/.test(artifact.sha256)
    || !Number.isSafeInteger(artifact.size_bytes)
    || artifact.size_bytes <= 0
    || artifact.size_bytes > MAX_SCREENSHOT_BYTES
    || !artifact.display_name
    || artifact.display_name === "."
    || artifact.display_name === ".."
    || path.basename(artifact.display_name) !== artifact.display_name
  ) {
    throw new Error("Room Environment screenshot artifact metadata is invalid")
  }
}
