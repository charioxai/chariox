import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { readFile } from "node:fs/promises"
import { test } from "node:test"

test("MP-08 MP-10 MP-11 provider status diagnostics discard probe output and preserve safe status", async () => {
  const source = await readFile(new URL("./provision-linux-docker-slice.sh", import.meta.url), "utf8")
  const body = source.slice(source.indexOf("print_provider_auth_status() {"), source.indexOf("provider_login_command() {"))
  const sentinel = "synthetic-provider-status-secret"
  const script = `
log() { printf '%s\\n' "$*"; }
exec_slice_with_timeout() {
  bash -c '
    timeout() { shift; "$@"; }
    codex() { printf "${sentinel}"; printf "${sentinel}" >&2; return 0; }
    opencode() { printf "${sentinel}"; printf "${sentinel}" >&2; return 1; }
    claude() { printf "${sentinel}"; printf "${sentinel}" >&2; return 124; }
  '"$4"
}
${body}
print_provider_auth_status
`
  const result = spawnSync("bash", ["-c", script], { encoding: "utf8", timeout: 3000, env: { PATH: process.env.PATH } })
  assert.equal(result.status, 0)
  const output = result.stdout + result.stderr
  assert(!output.includes(sentinel), "auth/status probe streams must not reach diagnostic capture")
  assert.match(output, /codex auth status probe completed/)
  assert.match(output, /opencode auth unavailable \(exit 1\)/)
  assert.match(output, /claude auth probe timed out/)
})
