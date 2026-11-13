import assert from "node:assert/strict"
import test from "node:test"
import { parseShellCommand } from "./shell-core.js"
import { executeConfigCommand } from "./shell-config-command.js"

test("offload setting defaults Off, writes the kernel and reads back its confirmed state", async () => {
  let enabled: boolean | undefined
  const requests: Record<string, unknown>[] = []
  const client = { send: async (request: Record<string, unknown>) => {
    requests.push(request)
    if (request.SetUserConfigValue) {
      const input = request.SetUserConfigValue as { path: string; value: string }
      assert.equal(input.path, "agents.offload_without_asking")
      enabled = input.value === "true"
      return { UserConfigUpdated: { path: "settings", config: { version: 1, agents: { offload_without_asking: enabled } } } }
    }
    return { UserConfig: { path: "settings", config: { version: 1, agents: { offload_without_asking: enabled } } } }
  } }
  const read = await executeConfigCommand(parseShellCommand("config agents-offload"), { client })
  assert.equal(read.ok, true)
  assert.match(read.message ?? "", /Agents may offload and acquire machines without asking: Off/)
  const on = await executeConfigCommand(parseShellCommand("config agents-offload on"), { client })
  assert.equal(on.ok, true)
  assert.match(on.message ?? "", /: On/)
  assert.equal(requests.length, 3)
  const off = await executeConfigCommand(parseShellCommand("config agents-offload off"), { client })
  assert.equal(off.ok, true)
  assert.match(off.message ?? "", /: Off/)
})
