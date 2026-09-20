import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { test } from "node:test"
import { fileURLToPath } from "node:url"
import path from "node:path"

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const provisionerPath = path.join(
  repositoryRoot,
  "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
)
const brokerPath = path.join(
  repositoryRoot,
  "apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
)

function section(source, startMarker, endMarker) {
  const start = source.indexOf(startMarker)
  const end = source.indexOf(endMarker, start + startMarker.length)
  assert.notEqual(start, -1, `missing section start: ${startMarker}`)
  assert.notEqual(end, -1, `missing section end: ${endMarker}`)
  return source.slice(start, end)
}

test("provisioner keeps noVNC as the default and selects only novnc or Selkies", async () => {
  const source = await readFile(provisionerPath, "utf8")
  assert.match(source, /SLICE_DISPLAY_BACKEND="\$\{CHARIOX_SLICE_DISPLAY_BACKEND:-novnc\}"/)
  assert.match(source, /SLICE_SELKIES_PORT="\$\{CHARIOX_SLICE_SELKIES_PORT:-\$SLICE_NOVNC_PORT\}"/)
  assert.match(source, /SLICE_SELKIES_HEALTH_TIMEOUT="\$\{CHARIOX_SLICE_SELKIES_HEALTH_TIMEOUT:-15\}"/)
  assert.match(source, /case "\$SLICE_DISPLAY_BACKEND" in[\s\S]*novnc[\s\S]*selkies[\s\S]*must be novnc or selkies/)
  assert.match(source, /selected_display_port\(\)[\s\S]*novnc\)[\s\S]*\$SLICE_NOVNC_PORT[\s\S]*selkies\)[\s\S]*\$SLICE_SELKIES_PORT/)
})

test("provisioner validates selected port and bounded Selkies health settings", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const validation = section(source, "validate_display_settings() {", "selected_display_port() {")
  assert.match(validation, /validate_display_port CHARIOX_SLICE_NOVNC_PORT/)
  assert.match(validation, /validate_display_port CHARIOX_SLICE_SELKIES_PORT/)
  assert.match(validation, /CHARIOX_SLICE_SELKIES_HEALTH_TIMEOUT must be numeric/)
  assert.match(validation, /SLICE_SELKIES_HEALTH_TIMEOUT\s+>=\s+1/)
  assert.match(validation, /SLICE_SELKIES_HEALTH_TIMEOUT\s+<=\s+120/)
  assert.match(source, /value >= 1 && value <= 65535/)
})

test("container creation publishes one selected host-loopback port and passes backend settings", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const create = section(source, "local docker_create_args=(", "    )\n    if [[ \"$SLICE_ALLOW_UNCONFINED_SECCOMP\"")
  assert.match(create, /-p "127\.0\.0\.1:\$display_port:\$display_port"/)
  assert.doesNotMatch(create, /-p "0\.0\.0\.0:/)
  assert.match(create, /-e "CHARIOX_SLICE_DISPLAY_BACKEND=\$SLICE_DISPLAY_BACKEND"/)
  assert.match(create, /-e "CHARIOX_SLICE_SELKIES_PORT=\$SLICE_SELKIES_PORT"/)
  assert.match(create, /-e "CHARIOX_SLICE_SELKIES_HEALTH_TIMEOUT=\$SLICE_SELKIES_HEALTH_TIMEOUT"/)
  assert.doesNotMatch(create, /SLICE_NOVNC_PORT:\$SLICE_NOVNC_PORT/)
})

test("exec and diagnostics carry only non-secret display settings", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const exec = section(source, 'run_with_timeout "$seconds" docker exec', "exec_slice() {")
  for (const name of [
    "CHARIOX_SLICE_DISPLAY_BACKEND",
    "CHARIOX_SLICE_NOVNC_PORT",
    "CHARIOX_SLICE_SELKIES_PORT",
    "CHARIOX_SLICE_SELKIES_HEALTH_TIMEOUT",
  ]) {
    assert.match(exec, new RegExp(`-e ${name}="`))
  }

  const diagnostics = section(source, "slice_screen_diagnostics() {", "run_required_phase() {")
  assert.match(diagnostics, /display backend:/)
  assert.match(diagnostics, /display host-loopback port:/)
  assert.match(diagnostics, /Selkies health timeout:/)
  assert.match(diagnostics, /\/opt\/chariox-selkies\/bin\/selkies/)
  assert.match(diagnostics, /\/opt\/chariox-slice\/logs\/selkies\.log/)
  assert.match(diagnostics, /\[REDACTED\]/)
  assert.match(diagnostics, /\[Bb\]\[Ee\]\[Aa\]\[Rr\]\[Ee\]\[Rr\]/)
  assert.doesNotMatch(diagnostics, /SLICE_RELAY_TOKEN|SLICE_CLOUD_RELAY_CONFIG/)
})

test("existing-container reprovision covers noVNC-to-Selkies and Selkies-to-noVNC transitions", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const mapping = section(
    source,
    "container_display_mapping_matches() {",
    "reconcile_existing_display_mapping() {",
  )
  const reconcile = section(
    source,
    "reconcile_existing_display_mapping() {",
    "if [[ ! \"$SLICE_ACCOUNT_OWNER\"",
  )
  const ensure = section(source, "ensure_container() {", "exec_slice_with_timeout() {")

  for (const transition of [
    { selectedBackend: "selkies", selectedPort: "SLICE_SELKIES_PORT", otherPort: "SLICE_NOVNC_PORT" },
    { selectedBackend: "novnc", selectedPort: "SLICE_NOVNC_PORT", otherPort: "SLICE_SELKIES_PORT" },
  ]) {
    assert.match(mapping, new RegExp(`SLICE_DISPLAY_BACKEND.*${transition.selectedBackend}`))
    assert.match(mapping, new RegExp(transition.selectedPort))
    assert.match(mapping, new RegExp(transition.otherPort))
  }
  assert.match(mapping, /selected_display_port\)/)
  assert.match(mapping, /HostIp/)
  assert.ok(mapping.includes("127\\\\.0\\\\.0\\\\.1"), "loopback matcher must stay literal")
  assert.match(mapping, /HostPort/)
  assert.match(mapping, /CHARIOX_SLICE_DISPLAY_BACKEND=selkies/)
  assert.match(mapping, /CHARIOX_SLICE_DISPLAY_BACKEND=novnc/)
  assert.match(reconcile, /mapping_status/)
  assert.match(reconcile, /docker rm -f "\$SLICE_NAME"/)
  assert.match(reconcile, /display backend\/port mapping changed/)
  assert.match(reconcile, /container still exists/)
  assert.match(reconcile, /cannot verify existing display backend\/port mapping/)
  assert.match(ensure, /reconcile_existing_display_mapping/)
})

test("broker exposes and validates the same display settings without changing protocol shapes", async () => {
  const source = await readFile(brokerPath, "utf8")
  for (const name of [
    "CHARIOX_SLICE_DISPLAY_BACKEND",
    "CHARIOX_SLICE_SELKIES_PORT",
    "CHARIOX_SLICE_SELKIES_HEALTH_TIMEOUT",
  ]) {
    assert.match(source, new RegExp(`\\"${name}\\"`))
  }
  assert.match(source, /\[\"novnc\", \"selkies\"\]/)
  assert.match(source, /SELKIES_HEALTH_TIMEOUT[\s\S]*Number\(environment\.CHARIOX_SLICE_SELKIES_HEALTH_TIMEOUT\) > 120/)
  assert.match(source, /SELKIES_PORT[\s\S]*Number\(environment\[name\]\) < minimum/)
  assert.doesNotMatch(source, /PROTOCOL_VERSION|protocol version/i)
})
