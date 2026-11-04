// MP-08 / MP-09 / MP-10 / MP-11 reach: write the `chariox-reach` tool list the kernel embeds from the CLI command table.
// Needs the built CLI (`pnpm --filter @chariox/cli build`).
import { readFileSync, writeFileSync } from "node:fs"
import { reachToolProblems, reachToolsDocument } from "../apps/cli/dist/agent-reach-tools.js"
const target = new URL("../apps/kernel/src/transport/runtime_tools/reach_tools.json", import.meta.url)
const problems = reachToolProblems()
if (problems.length) throw new Error(problems.join("\n"))
const content = JSON.stringify(reachToolsDocument(), null, 2) + "\n"
if (process.argv.includes("--check")) {
  if (readFileSync(target, "utf8") !== content) throw new Error("reach_tools.json is stale; run pnpm reach:generate")
} else writeFileSync(target, content)
