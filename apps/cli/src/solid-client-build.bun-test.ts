import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import test from "node:test"
import { fileURLToPath } from "node:url"

const run = (entry: string) => spawnSync(process.execPath, [
  fileURLToPath(new URL(`../dist/${entry}`, import.meta.url)), "logs", "--help",
], { encoding: "utf8" })

test("the CLI entry loads Solid's client build", () => {
  const entry = run("index.js")
  assert.equal(entry.status, 0, entry.stderr)
  assert.match(entry.stdout, /usage: chariox-cli logs/)
})

test("the CLI refuses to start on Solid's server build", () => {
  const bare = run("cli-main.js")
  assert.notEqual(bare.status, 0)
  assert.match(bare.stderr, /solid-js resolved to its server build/)
})
