import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { readFile } from "node:fs/promises"
import test from "node:test"

const provisioner = new URL("./provision-linux-docker-slice.sh", import.meta.url)

test("slice failure diagnostics redact complete Authorization values", async () => {
  const source = await readFile(provisioner, "utf8")
  const start = source.indexOf("slice_screen_diagnostics() {")
  const end = source.indexOf("\nrun_required_phase() {", start)
  assert.ok(start >= 0 && end > start)

  const secret = "chariox-diagnostic-redaction-sentinel"
  const harness = [
    "set -Eeuo pipefail",
    "log() { :; }",
    "run_with_timeout() { shift; \"$@\"; }",
    "docker() { printf '%s\\n' \"Authorization: Digest ${DIAG_SECRET}\" \"token=${DIAG_SECRET} trailing-value\" >&2; local script; for script; do :; done; bash -c \"$script\"; }",
    "tail() { printf '%s\\n' \"Authorization: Bearer ${DIAG_SECRET}\" \"authorization: Basic ${DIAG_SECRET}\" \"Authorization: Digest ${DIAG_SECRET}\" 'diagnostic-preserved-text'; }",
    "export -f tail",
    "SLICE_NAME=diagnostic-test",
    source.slice(start, end),
    "slice_screen_diagnostics",
  ].join("\n")
  const result = spawnSync("bash", ["-c", harness], {
    encoding: "utf8",
    env: { PATH: process.env.PATH ?? "/usr/bin:/bin", DIAG_SECRET: secret },
  })
  assert.equal(result.status, 0, result.stderr)
  const output = result.stdout + result.stderr
  assert.match(output, /Authorization: \[REDACTED\]/i)
  assert.match(output, /token=\[REDACTED\]/i)
  assert.match(output, /diagnostic-preserved-text/)
  assert.equal(output.includes(secret), false)
})
