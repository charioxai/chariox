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

test("offload limits change one key each, unset back to the defaults, and the summary names them", async () => {
  const agents: Record<string, unknown> = { offload_without_asking: true }
  const requests: Record<string, unknown>[] = []
  const client = { send: async (request: Record<string, unknown>) => {
    requests.push(request)
    const set = request.SetUserConfigValue as { path: string; value: string } | undefined
    const unset = request.UnsetUserConfigValue as { path: string } | undefined
    if (set) agents[set.path.replace("agents.", "")] = set.path.endsWith("classes") ? set.value.split(",") : set.path.endsWith("asking") ? set.value === "true" : Number(set.value)
    if (unset) delete agents[unset.path.replace("agents.", "")]
    const config = { version: 1, agents: { ...agents } }
    return set || unset ? { UserConfigUpdated: { path: "settings", config } } : { UserConfig: { path: "settings", config } }
  } }
  const run = (line: string) => executeConfigCommand(parseShellCommand(line), { client })
  assert.match((await run("config agents-offload")).message ?? "", /On \(classes: the smallest offered; at most 4 at the same time; no daily cap\)/)
  await run("config agents-offload classes small,medium")
  await run("config agents-offload max 2")
  const capped = await run("config agents-offload cap 5.25")
  assert.deepEqual(requests.filter(r => r.SetUserConfigValue).map(r => r.SetUserConfigValue), [
    { path: "agents.offload_classes", value: "small,medium" },
    { path: "agents.offload_max_machines", value: "2" },
    { path: "agents.offload_daily_cap_micros", value: "5250000" },
  ])
  assert.match(capped.message ?? "", /classes: small, medium; at most 2 at the same time; at most 5.25 per day/)
  await run("config agents-offload classes smallest")
  await run("config agents-offload max default")
  const reset = await run("config agents-offload cap none")
  assert.match(reset.message ?? "", /classes: the smallest offered; at most 4 at the same time; no daily cap/)
  for (const bad of ["config agents-offload cap 5.1234567", "config agents-offload cap -1", "config agents-offload max", "config agents-offload fast"]) {
    const refused = await run(bad)
    assert.equal(refused.ok, false, bad)
    assert.match(refused.message ?? "", /usage: config agents-offload/)
  }
})
