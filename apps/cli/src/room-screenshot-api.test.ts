import assert from "node:assert/strict"
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { downloadRoomEnvironmentScreenshot } from "./room-screenshot-api.js"

test("downloadRoomEnvironmentScreenshot materializes bounded chunks on the TUI host", async () => {
  const outputRoot = await mkdtemp(join(tmpdir(), "chariox-room-screenshot-"))
  const requests: unknown[] = []
  try {
    const result = await downloadRoomEnvironmentScreenshot({
      sessionId: "session-1",
      attachmentId: "attachment-1",
      outputRoot,
      send: async <TResponse>(request: unknown) => {
        requests.push(request)
        if (Object.prototype.hasOwnProperty.call(request, "CaptureRoomEnvironmentScreenshot")) {
          return {
            RoomEnvironmentScreenshotCaptured: {
              artifact: {
                artifact_id: "artifact-1",
                sha256: "bef57ec7f53a6d40beb640a780a639c83bc29ac8a9816f1fc6c5c6dcd93c4721",
                size_bytes: 6,
                media_type: "image/png",
                display_name: "capture.png",
              },
            },
          } as TResponse
        }
        const offset = (request as { ReadRoomEnvironmentScreenshotChunk: { offset: number } })
          .ReadRoomEnvironmentScreenshotChunk.offset
        return {
          RoomEnvironmentScreenshotChunk: {
            chunk: {
              artifact_id: "artifact-1",
              offset,
              data_base64: Buffer.from(offset === 0 ? "abc" : "def").toString("base64"),
              eof: offset === 3,
            },
          },
        } as TResponse
      },
    })

    assert.equal(result.path, join(outputRoot, "capture.png"))
    assert.equal((await readFile(result.path)).toString(), "abcdef")
    assert.deepEqual(requests, [
      {
        CaptureRoomEnvironmentScreenshot: {
          session_id: "session-1",
          attachment_id: "attachment-1",
        },
      },
      {
        ReadRoomEnvironmentScreenshotChunk: {
          session_id: "session-1",
          attachment_id: "attachment-1",
          artifact_id: "artifact-1",
          offset: 0,
          max_bytes: 131_072,
        },
      },
      {
        ReadRoomEnvironmentScreenshotChunk: {
          session_id: "session-1",
          attachment_id: "attachment-1",
          artifact_id: "artifact-1",
          offset: 3,
          max_bytes: 131_072,
        },
      },
    ])
  } finally {
    await rm(outputRoot, { recursive: true, force: true })
  }
})

test("downloadRoomEnvironmentScreenshot removes a partial file after a digest mismatch", async () => {
  const outputRoot = await mkdtemp(join(tmpdir(), "chariox-room-screenshot-digest-"))
  try {
    await assert.rejects(
      downloadRoomEnvironmentScreenshot({
        sessionId: "session-1",
        attachmentId: "attachment-1",
        outputRoot,
        send: async <TResponse>(request: unknown) => {
          if (Object.prototype.hasOwnProperty.call(request, "CaptureRoomEnvironmentScreenshot")) {
            return {
              RoomEnvironmentScreenshotCaptured: {
                artifact: {
                  artifact_id: "artifact-1",
                  sha256: "0000000000000000000000000000000000000000000000000000000000000000",
                  size_bytes: 3,
                  media_type: "image/png",
                  display_name: "capture.png",
                },
              },
            } as TResponse
          }
          return {
            RoomEnvironmentScreenshotChunk: {
              chunk: {
                artifact_id: "artifact-1",
                offset: 0,
                data_base64: Buffer.from("abc").toString("base64"),
                eof: true,
              },
            },
          } as TResponse
        },
      }),
      /digest verification failed/,
    )
    assert.deepEqual(await readdir(outputRoot), [])
  } finally {
    await rm(outputRoot, { recursive: true, force: true })
  }
})

test("downloadRoomEnvironmentScreenshot rejects a stalled offset and removes its partial file", async () => {
  const outputRoot = await mkdtemp(join(tmpdir(), "chariox-room-screenshot-offset-"))
  try {
    await assert.rejects(
      downloadRoomEnvironmentScreenshot({
        sessionId: "session-1",
        attachmentId: "attachment-1",
        outputRoot,
        send: async <TResponse>(request: unknown) => {
          if (Object.prototype.hasOwnProperty.call(request, "CaptureRoomEnvironmentScreenshot")) {
            return {
              RoomEnvironmentScreenshotCaptured: {
                artifact: {
                  artifact_id: "artifact-1",
                  sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                  size_bytes: 3,
                  media_type: "image/png",
                  display_name: "capture.png",
                },
              },
            } as TResponse
          }
          return {
            RoomEnvironmentScreenshotChunk: {
              chunk: {
                artifact_id: "artifact-1",
                offset: 1,
                data_base64: Buffer.from("abc").toString("base64"),
                eof: true,
              },
            },
          } as TResponse
        },
      }),
      /does not match the requested artifact offset/,
    )
    assert.deepEqual(await readdir(outputRoot), [])
  } finally {
    await rm(outputRoot, { recursive: true, force: true })
  }
})

test("downloadRoomEnvironmentScreenshot rejects oversized metadata before requesting chunks", async () => {
  const outputRoot = await mkdtemp(join(tmpdir(), "chariox-room-screenshot-size-"))
  let requests = 0
  try {
    await assert.rejects(
      downloadRoomEnvironmentScreenshot({
        sessionId: "session-1",
        attachmentId: "attachment-1",
        outputRoot,
        send: async <TResponse>() => {
          requests += 1
          return {
            RoomEnvironmentScreenshotCaptured: {
              artifact: {
                artifact_id: "artifact-1",
                sha256: "0000000000000000000000000000000000000000000000000000000000000000",
                size_bytes: 64 * 1024 * 1024 + 1,
                media_type: "image/png",
                display_name: "capture.png",
              },
            },
          } as TResponse
        },
      }),
      /artifact metadata is invalid/,
    )
    assert.equal(requests, 1)
    assert.deepEqual(await readdir(outputRoot), [])
  } finally {
    await rm(outputRoot, { recursive: true, force: true })
  }
})

test("downloadRoomEnvironmentScreenshot rejects dot-segment display names before writing", async () => {
  const outputRoot = await mkdtemp(join(tmpdir(), "chariox-room-screenshot-name-"))
  try {
    await assert.rejects(
      downloadRoomEnvironmentScreenshot({
        sessionId: "session-1",
        attachmentId: "attachment-1",
        outputRoot,
        send: async <TResponse>() => ({
          RoomEnvironmentScreenshotCaptured: {
            artifact: {
              artifact_id: "artifact-1",
              sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
              size_bytes: 3,
              media_type: "image/png",
              display_name: "..",
            },
          },
        }) as TResponse,
      }),
      /artifact metadata is invalid/,
    )
    assert.deepEqual(await readdir(outputRoot), [])
  } finally {
    await rm(outputRoot, { recursive: true, force: true })
  }
})
