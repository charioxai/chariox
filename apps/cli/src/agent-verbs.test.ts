import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { dirname, join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { COMMANDS } from "./agent-command-table.js"
import { AGENT_VERB_NAMES } from "./agent-verbs.js"

test("every command of the agent guide is an agent verb", () => {
  for (const spec of COMMANDS) {
    const verb = spec.name.split(" ")[0]!
    assert.ok(verb === "sudo" || AGENT_VERB_NAMES.includes(verb), `${spec.name} would load the TUI before it runs`)
  }
})

// p8 measured 145 ms of every agent call in the TUI module graph that no verb uses.
test("an agent verb answers without loading the rendering stack", () => {
  const here = dirname(fileURLToPath(import.meta.url))
  const script = `
    const seen = []
    Bun.plugin({ name: "record", setup(build) { build.onResolve({ filter: /.*/ }, (args) => { seen.push(args.path); return undefined }) } })
    const { runAgentVerb } = await import(${JSON.stringify(join(here, "agent-verbs.js"))})
    const quiet = process.stdout.write.bind(process.stdout)
    process.stdout.write = () => true
    const handled = await runAgentVerb(["guide"])
    process.stdout.write = quiet
    console.log(JSON.stringify({ handled, tui: seen.filter((path) => /opentui|solid-js|cli-main|cli-app-root/.test(path)) }))
  `
  const run = spawnSync("bun", ["-e", script], { encoding: "utf8" })
  assert.equal(run.status, 0, run.stderr)
  const result = JSON.parse(run.stdout.trim().split("\n").pop()!) as { handled: boolean; tui: string[] }
  assert.equal(result.handled, true)
  assert.deepEqual(result.tui, [])
})
