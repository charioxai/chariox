#!/usr/bin/env node
import { readFileSync, writeFileSync } from "node:fs"

const registry = new URL("../apps/kernel/src/runtime/state/runtime_interaction_owned_state/card_registry.rs", import.meta.url)
const document = new URL("../docs/RUNTIME_CARD_RULES.md", import.meta.url)
const rows = [
  "| Group | Trigger | Live state | Answerer | Retirement | Cold restart |",
  "| --- | --- | --- | --- | --- | --- |",
]
const source = readFileSync(registry, "utf8")
for (const line of source.match(/\ncard_kinds! \{\n([^}]+)\}/)?.[1].trimEnd().split("\n") ?? []) {
  const fields = JSON.parse(line.match(/^\s*\w+ => (\[.+\]),$/)?.[1] ?? "null")
  if (!fields || fields.length !== 6) throw new Error(`Invalid registry row: ${line}`)
  rows.push(`| ${fields.join(" | ")} |`)
}
if (rows.length === 2) throw new Error("No card kinds found in the registry")
const generated = rows.join("\n") + "\n"
if (process.argv.includes("--check")) {
  if (readFileSync(document, "utf8") !== generated) {
    throw new Error("Run node scripts/generate-runtime-card-rules.mjs to update the card table")
  }
} else {
  writeFileSync(document, generated)
}
