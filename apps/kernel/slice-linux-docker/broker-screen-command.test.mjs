import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import test from "node:test"
import { fileURLToPath } from "node:url"

const broker = fileURLToPath(new URL("./managed-docker-broker.mjs", import.meta.url))
const container = "chariox-slice-0123456789abcdef"

function screenCommand(action, roomSessionId) {
  return [
    "exec",
    "-e", "CHARIOX_SLICE_VIEWER_BACKEND=selkies",
    "-e", "CHARIOX_SLICE_NOVNC_PORT=6080",
    "-e", "CHARIOX_SLICE_DISPLAY_MODE=headed",
    ...(roomSessionId ? ["-e", `CHARIOX_ROOM_ENVIRONMENT_SESSION_ID=${roomSessionId}`] : []),
    "-u", "slice", container, "/opt/chariox-slice/slice-screen.sh", action,
  ]
}

function validate(args) {
  return spawnSync(process.execPath, [broker, "--validate-request"], {
    encoding: "utf8",
    input: JSON.stringify({ kind: "docker", args }),
  })
}

// The kernel runs these exact commands when it stops the desktop before a snapshot
// and starts it again on resume; the broker must accept the Room-bound shape.
for (const action of ["start", "stop", "status"]) {
  test(`broker accepts the ${action} screen command with and without a bound Room`, () => {
    for (const roomSessionId of [undefined, "room-session_1"]) {
      const result = validate(screenCommand(action, roomSessionId))
      assert.equal(result.status, 0, result.stderr)
    }
  })
}

test("broker rejects a malformed Room session id or a second extra environment field", () => {
  for (const args of [
    screenCommand("stop", "room session;rm"),
    screenCommand("stop", "x".repeat(181)),
    [...screenCommand("stop", "room-1").slice(0, 8), "-e", "CHARIOX_EXTRA=1", ...screenCommand("stop", "room-1").slice(8)],
  ]) {
    const result = validate(args)
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /Docker screen/)
  }
})
