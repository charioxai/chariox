import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import path from "node:path"
import test from "node:test"

// MP-08 / MP-10 / MP-11: exercise the built native CLI argument boundary.
const client = path.resolve("dist/index.js")
for (const provider of ["codex", "claude", "opencode"]) {
  test(`MP-08/MP-11 native ${provider} accepts relay authorization from an environment variable`, () => {
    const result = spawnSync("bun", [client, provider, "--relay-token-env", "INTEG2_TEST_RELAY_AUTH", "--help"], {
      encoding: "utf8", env: { ...process.env, INTEG2_TEST_RELAY_AUTH: "synthetic-noncredential-test-value" },
    })
    assert.equal(result.status, 0, result.stderr)
    assert.match(result.stdout, /usage:/i)
    assert.doesNotMatch(result.stdout + result.stderr, /synthetic-noncredential-test-value/)
  })
}

for (const provider of ["codex", "claude", "opencode"]) {
  for (const name of ["BAD-NAME", "INTEG2_UNSET_RELAY_AUTH"]) {
    test(`MP-11 native ${provider} rejects invalid or missing relay environment ${name}`, () => {
      const env = { ...process.env }
      delete env.INTEG2_UNSET_RELAY_AUTH
      const result = spawnSync("bun", [client, provider, "--relay-token-env", name, "--help"], { encoding: "utf8", env })
      assert.equal(result.status, 1)
      assert.match(result.stderr, /--relay-token-env/)
    })
  }
  test(`MP-11 native ${provider} rejects ambiguous relay authorization sources`, () => {
    const result = spawnSync("bun", [client, provider, "--relay-token", "synthetic-first", "--relay-token-env", "INTEG2_TEST_RELAY_AUTH", "--help"], {
      encoding: "utf8", env: { ...process.env, INTEG2_TEST_RELAY_AUTH: "synthetic-second" },
    })
    assert.equal(result.status, 1)
    assert.match(result.stderr, /cannot be combined/)
    assert.doesNotMatch(result.stdout + result.stderr, /synthetic-first|synthetic-second/)
  })
}
