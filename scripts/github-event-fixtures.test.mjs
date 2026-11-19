// The GitHub generator's webhook fixtures and the producer allow-list agree on one list of event types.
import assert from "node:assert/strict"
import { readFileSync, readdirSync } from "node:fs"
import { dirname, join } from "node:path"
import { test } from "node:test"
import { fileURLToPath } from "node:url"

const root = join(dirname(fileURLToPath(import.meta.url)), "..")
const fixtureDir = join(root, "docs/fixtures/event-generators/github")
const index = JSON.parse(readFileSync(join(fixtureDir, "index.json"), "utf8"))

test("every fixture is indexed once and is a payload of its repository", () => {
  const files = readdirSync(fixtureDir).filter((name) => name !== "index.json").sort()
  assert.deepEqual(index.map((entry) => entry.fixture).sort(), files)
  for (const entry of index) {
    const payload = JSON.parse(readFileSync(join(fixtureDir, entry.fixture), "utf8"))
    assert.equal(payload.repository.full_name, "charioxai/chariox-github-app-drill", entry.fixture)
    assert.ok(entry.event_type.startsWith(entry.github_event), entry.fixture)
  }
})

test("the producer allow-list names exactly the event types the fixtures cover", () => {
  const script = readFileSync(join(root, "deploy/event-publication/prepare-secrets.sh"), "utf8")
  const line = script.split("\n").find((candidate) => candidate.startsWith("printf '{"))
  assert.ok(line, "capabilities printf not found")
  const capabilities = JSON.parse(line.slice("printf '".length, line.indexOf("\\n'")).replaceAll("%s", "token"))
  assert.deepEqual(
    [...capabilities["dev.chariox.github"].event_types].sort(),
    [...new Set(index.map((entry) => entry.event_type))].sort(),
  )
})
