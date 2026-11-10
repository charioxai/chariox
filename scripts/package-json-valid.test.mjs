import test from "node:test"
import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { readFileSync } from "node:fs"

test("every tracked package.json is valid JSON", () => {
  const files = execFileSync("git", ["ls-files", "package.json", "**/package.json"], { encoding: "utf8" }).split("\n").filter(Boolean)
  assert.ok(files.length > 0, "git lists package.json files")
  for (const file of files) {
    assert.doesNotThrow(() => JSON.parse(readFileSync(file, "utf8")), `${file} parses as JSON`)
  }
})
