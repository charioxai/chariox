import assert from "node:assert/strict"
import test from "node:test"

import {
  forgetGatewayKernelRequest,
  joinKernelLinkRequest,
  joinTerminalPairingLinkRequest,
  listGatewayKernelsRequest,
} from "./ipc-remote-connection-requests.js"

test("terminal pairing join sends the receiving CLI public thumbprint", () => {
  assert.deepEqual(joinTerminalPairingLinkRequest(
    "chariox-terminal-pair-v1.fixture",
    "terminal-1",
    "cli",
    null,
    "cli-thumbprint",
  ), {
    JoinTerminalPairingLink: {
      pairing_link: "chariox-terminal-pair-v1.fixture",
      terminal_id: "terminal-1",
      terminal_type: "cli",
      alias: null,
      public_key_thumbprint: "cli-thumbprint",
    },
  })
})

test("legacy terminal join omits a public thumbprint and receives no viewer authority", () => {
  assert.deepEqual(joinTerminalPairingLinkRequest("chariox-terminal-pair-v1.fixture"), {
    JoinTerminalPairingLink: {
      pairing_link: "chariox-terminal-pair-v1.fixture",
      terminal_id: null,
      terminal_type: null,
      alias: null,
    },
  })
})

test("machine link requests carry the relay token only when one is given", () => {
  assert.deepEqual(joinKernelLinkRequest("chariox-terminal-pair-v1.fixture"), {
    JoinKernelLink: { link: "chariox-terminal-pair-v1.fixture" },
  })
  assert.deepEqual(joinKernelLinkRequest("chariox-terminal-pair-v1.fixture", "client-token"), {
    JoinKernelLink: { link: "chariox-terminal-pair-v1.fixture", relay_token: "client-token" },
  })
  assert.deepEqual(forgetGatewayKernelRequest("builder"), { ForgetGatewayKernel: { name: "builder" } })
  assert.deepEqual(listGatewayKernelsRequest(), { ListGatewayKernels: {} })
})
