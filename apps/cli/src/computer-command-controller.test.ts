// MP-08 / MP-11: human commands use the selected terminal transport.
import test from "node:test"
import assert from "node:assert/strict"
import { createComputerCommandController } from "./computer-command-controller.js"
import { parseSlashCommand } from "./commands.js"
test("MP-08 Computer parser has an exact command boundary", () => {
  assert.equal(parseSlashCommand("/computer takeover")?.kind, "computer")
  assert.equal(parseSlashCommand("/computerized"), null)
})
test("MP-11 takeover gets a fresh desktop and stays on the selected terminal", async () => {
  const calls: unknown[] = [], lines: string[] = []
  const client = { send: async <T>(request: unknown): Promise<T> => { calls.push(request); return {KernelBrowser:{result:calls.length===1?{surface_id:"s",generation:"g"}:{owner_actor_id:"terminal:human"}}} as T } }
  await createComputerCommandController({client,appendNotice:m=>lines.push(m)}).handle(["takeover"])
  assert.deepEqual(calls, [{KernelBrowser:{command:{op:"computer",command:{op:"state"}}}}, {KernelBrowser:{command:{op:"computer",command:{op:"takeover",target:{surface_id:"s",generation:"g"}}}}}])
  assert.match(lines[0]!, /paused/i)
})
test("MP-11 invalid command fails before sending", async () => {
  await assert.rejects(createComputerCommandController({client:{send:async()=>assert.fail()},appendNotice:()=>{}}).handle(["unknown"]), /Usage/)
})
