import assert from "node:assert/strict"
import test from "node:test"

import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { nativeProviderInteractionMinimumProtocolVersion, requestNativeProviderInteractionRequest } from "./ipc-terminal-runtime-requests.js"

test("protocol 405 native approval requests require the originating prompt and run", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 479)
  assert.equal(nativeProviderInteractionMinimumProtocolVersion, 405)
  const origin = { scope: "prompt", prompt_id: "prompt-A", provider_run_id: "run-A" } as const
  const request = requestNativeProviderInteractionRequest("session", "agent", "interaction", "Allow?", "Allow?", origin)
  assert.deepEqual(request.RequestNativeProviderTurnInteraction.origin, origin)
  assert.equal(request.RequestNativeProviderTurnInteraction.default_on_timeout, "deny")
})
