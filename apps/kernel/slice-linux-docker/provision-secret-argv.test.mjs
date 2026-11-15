import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

const script = readFileSync(new URL("./docker/start-runtime.sh", import.meta.url), "utf8")
const relayStart = script.indexOf('if [[ -z "${CHARIOX_SLICE_RELAY_URL:-}" ]]; then')
const relay = script.slice(relayStart, script.indexOf("\nfi", relayStart) + 3)
const kernelStart = script.indexOf("start_slice_kernel() {")
const kernel = script.slice(kernelStart, script.indexOf("\n}\n", kernelStart) + 3)
assert.ok(relayStart >= 0 && kernelStart >= 0)

for (const [cloud, external] of [[false, false], [false, true], [true, false], [true, true]]) {
  test(`runtime screen launch keeps credentials out of argv (Cloud config ${cloud}, external relay ${external})`, () => {
    const scratch = mkdtempSync(join(tmpdir(), "chariox-secret-argv-"))
    try {
      const result = spawnSync("bash", ["-c", `
set -Eeuo pipefail
${script.slice(script.indexOf('ROOT="'), script.indexOf('mkdir -p "$LOGS"'))}
mkdir -p "$LOGS" "$PRIVATE_RUNTIME_ROOT"
KERNEL_LOCAL_AUTH_FILE="$PRIVATE_RUNTIME_ROOT/auth.marker"
default_provider_env=()
kernel_relay_env=()
# Include the base's argv array so this drill also detects that kernel leak.
if [[ -z "$CLOUD_RELAY_CONFIG_JSON" ]]; then
  kernel_relay_env=(CHARIOX_RELAY_URL="$RELAY_URL" CHARIOX_RELAY_TOKEN="$RELAY_TOKEN")
fi
screen() {
  local argument session=unknown
  for argument in "$@"; do
    if [[ "$argument" == *"$RELAY_TOKEN"* || "$argument" == CHARIOX_KERNEL_LOCAL_AUTH_TOKEN=* ]]; then
      printf 'credential reached screen argv\\n' >&2; return 1
    fi
    case "$argument" in chariox-slice-relay|chariox-slice-kernel) session="$argument";; esac
  done
  if [[ "$session" == chariox-slice-relay || -z "$CLOUD_RELAY_CONFIG_JSON" ]]; then
    [[ "\${CHARIOX_RELAY_TOKEN:-}" == "$RELAY_TOKEN" ]] || return 2
    if [[ "$session" == chariox-slice-kernel ]]; then
      [[ "\${CHARIOX_RELAY_URL:-}" == "$RELAY_URL" ]] || return 3
    fi
  else
    [[ -z "\${CHARIOX_RELAY_TOKEN:-}" ]] || return 4
  fi
  printf '%s safe\\n' "$session"
}
sleep() { :; }
wait_for_screen_session() { :; }
wait_for_kernel_auth_consumption() { :; }
dd() { printf dummy-local-auth; }
${relay}
${kernel}
start_slice_kernel
[[ -z "\${CHARIOX_RELAY_TOKEN:-}" ]] # exported only within each launch
`], {
        env: { PATH: process.env.PATH, HOME: scratch, CHARIOX_SLICE_ROOT: scratch,
          CHARIOX_HOME: join(scratch, "home"), CHARIOX_SLICE_RELAY_TOKEN: "dummy-relay-canary",
          CHARIOX_SLICE_CLOUD_RELAY_CONFIG_JSON: cloud ? "{}" : "",
          ...(external ? { CHARIOX_SLICE_RELAY_URL: "ws://relay.example:43130" } : {}) },
        encoding: "utf8", timeout: 5000,
      })
      assert.equal(result.status, 0, result.stderr)
      assert.equal(result.stdout.includes("chariox-slice-relay safe"), !external)
      assert.match(result.stdout, /chariox-slice-kernel safe/)
    } finally { rmSync(scratch, { recursive: true, force: true }) }
  })
}
