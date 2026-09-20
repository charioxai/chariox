import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
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

test("failure diagnostics redact complete Authorization values through the execution path", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const diagnostics = section(source, "slice_screen_diagnostics() {", "run_required_phase() {")
  const requiredPhase = section(source, "run_required_phase() {", "copy_provider_auth_file() {")
  const secret = "chariox-authorization-redaction-sentinel-20260920"
  const base64Secret = Buffer.from(secret, "utf8").toString("base64")
  const urlEncodedSecret = encodeURIComponent(secret)
  const harness = [
    "set -Eeuo pipefail",
    "log() { printf '[harness] %s\\n' \"$*\" >&2; }",
    "selected_display_port() { printf '%s\\n' \"$SLICE_SELKIES_PORT\"; }",
    "run_with_timeout() { local seconds=\"$1\"; shift; \"$@\"; }",
    "tail() { printf '%s\\n' \"Authorization: Bearer ${DIAG_SECRET}\" \"authorization: Basic ${DIAG_SECRET}\" \"Authorization: Digest ${DIAG_SECRET}\" \"Authorization: Bearer ${DIAG_BASE64}\" \"Authorization: Basic ${DIAG_URL}\" 'diagnostic-preserved-text'; }",
    "export -f tail",
    "docker() {",
    "  [[ \"$1\" == exec ]] || return 0",
    "  local script=",
    "  for script; do :; done",
    "  bash -c \"$script\"",
    "}",
    "SLICE_NAME=sentinel-container",
    "SLICE_DISPLAY_BACKEND=selkies",
    "SLICE_NOVNC_PORT=6080",
    "SLICE_SELKIES_PORT=6081",
    "SLICE_SELKIES_HEALTH_TIMEOUT=15",
    diagnostics,
    requiredPhase,
    "if run_required_phase desktop false; then phase_status=0; else phase_status=$?; fi",
    "printf 'PHASE_STATUS=%s\\n' \"$phase_status\"",
  ].join("\n")
  const result = spawnSync("bash", ["-c", harness], {
    env: {
      ...process.env,
      DIAG_SECRET: secret,
      DIAG_BASE64: base64Secret,
      DIAG_URL: urlEncodedSecret,
    },
    encoding: "utf8",
  })
  assert.equal(result.status, 0, result.stderr)
  const output = `${result.stdout}${result.stderr}`
  assert.match(output, /PHASE_STATUS=1/)
  assert.match(output, /Authorization: \[REDACTED\]/i)
  assert.match(output, /diagnostic-preserved-text/)
  for (const credential of [secret, base64Secret, urlEncodedSecret]) {
    assert.equal(output.includes(credential), false, `diagnostic leaked credential variant: ${credential}`)
  }
})

function runSavedStateProbe(source, mode) {
  const runtimeCompatible = section(source, "image_runtime_compatible() {", "image_selkies_capable() {")
  const selkiesCapable = section(source, "image_selkies_capable() {", "require_saved_state_compatibility() {")
  const compatibility = section(source, "require_saved_state_compatibility() {", "build_standard_runtime_image() {")
  const restore = section(source, "restore_saved_home_volume() {", "machine_id_hex() {")
  return spawnSync(
    "bash",
    [
      "-c",
      [
        "set -Eeuo pipefail",
        "log() { printf '[probe] %s\\n' \"$*\" >&2; }",
        "fail() { printf 'FAIL %s\\n' \"$*\" >&2; return 91; }",
        "run_with_timeout() { local seconds=\"$1\"; shift; \"$@\"; }",
        "docker() {",
        "  printf 'DOCKER_CALL %s\\n' \"$*\" >&2",
        "  [[ \"$1\" == image && \"$2\" == inspect ]] || return 0",
        "  local format image capable=0",
        "  if [[ \"$3\" == -f ]]; then format=\"$4\"; image=\"$5\"; else image=\"$3\"; fi",
        "  case \"$image\" in",
        "    saved-current) [[ \"$PROBE_MODE\" == compatible ]] && capable=1 ;;",
        "    current-base) capable=1 ;;",
        "    legacy-saved) capable=0 ;;",
        "    missing-base) return 1 ;;",
        "    *) return 1 ;;",
        "  esac",
        "  [[ \"$3\" == -f ]] || return 0",
        "  case \"$format\" in",
        "    *io.chariox.relay-peer-protocol-version*) printf '9\\n' ;;",
        "    *io.chariox.runtime-source-revision*) printf 'runtime-current\\n' ;;",
        "    *io.chariox.selkies-version*) (( capable )) && printf '0.0.0.dev0\\n' || printf '<no value>\\n' ;;",
        "    *io.chariox.selkies-source-revision*) (( capable )) && printf '0123456789012345678901234567890123456789\\n' || printf '<no value>\\n' ;;",
        "    *io.chariox.selkies-source*) (( capable )) && printf 'https://github.com/selkies-project/selkies/commit/0123456789012345678901234567890123456789\\n' || printf '<no value>\\n' ;;",
        "    *io.chariox.selkies-license*) (( capable )) && printf 'MPL-2.0\\n' || printf '<no value>\\n' ;;",
        "    *) printf '<no value>\\n' ;;",
        "  esac",
        "}",
        "SLICE_DISPLAY_BACKEND=selkies",
        "SLICE_SAVED_HOME_ARCHIVE=/dev/null",
        "SLICE_HOME_VOLUME=saved-home",
        "SLICE_NAME=saved-state-probe",
        "SLICE_IMAGE=\"${PROBE_SAVED_IMAGE}\"",
        "SLICE_BASE_IMAGE=\"${PROBE_BASE_IMAGE}\"",
        "SLICE_RELAY_PEER_PROTOCOL_VERSION=9",
        "SLICE_RUNTIME_SOURCE_REVISION=runtime-current",
        "SLICE_BUILD_IMAGE=never",
        runtimeCompatible,
        selkiesCapable,
        compatibility,
        restore,
        "if require_saved_state_compatibility; then status=0; else status=$?; fi",
        "printf 'STATUS=%s\\nIMAGE=%s\\n' \"$status\" \"$SLICE_IMAGE\"",
        "if [[ \"$PROBE_MODE\" == legacy ]]; then",
        "  if restore_saved_home_volume; then restore_status=0; else restore_status=$?; fi",
        "  printf 'RESTORE_STATUS=%s\\n' \"$restore_status\"",
        "fi",
      ].join("\n"),
    ],
    {
      env: {
        ...process.env,
        PROBE_MODE: mode,
        PROBE_SAVED_IMAGE: mode === "compatible" ? "saved-current" : "legacy-saved",
        PROBE_BASE_IMAGE: mode === "rejected" ? "missing-base" : "current-base",
      },
      encoding: "utf8",
    },
  )
}

function runSavedStatePolicyProbe(source, policy, baseState) {
  const runtimeCompatible = section(source, "image_runtime_compatible() {", "image_selkies_capable() {")
  const selkiesCapable = section(source, "image_selkies_capable() {", "ensure_saved_state_capable_base() {")
  const basePreflight = section(source, "ensure_saved_state_capable_base() {", "require_saved_state_compatibility() {")
  const compatibility = section(source, "require_saved_state_compatibility() {", "build_standard_runtime_image() {")
  const buildStandard = section(source, "build_standard_runtime_image() {", "ensure_runtime_base_image() {")
  const buildImage = section(source, "build_image() {", "refresh_saved_state_runtime() {")
  return spawnSync(
    "bash",
    [
      "-c",
      [
        "set -Eeuo pipefail",
        "log() { printf '[probe] %s\\n' \"$*\" >&2; }",
        "fail() { printf 'FAIL %s\\n' \"$*\" >&2; return 91; }",
        "run_with_timeout() { local seconds=\"$1\"; shift; \"$@\"; }",
        "docker() {",
        "  printf 'DOCKER_CALL %s\\n' \"$*\" >&2",
        "  if [[ \"$1\" == build ]]; then",
        "    printf 'DOCKER_BUILD %s\\n' \"$*\" >&2",
        "    if [[ \"$PROBE_BUILD_RESULT\" == capable ]]; then PROBE_BASE_STATE=capable; return 0; fi",
        "    return 42",
        "  fi",
        "  if [[ \"$1\" != image || \"$2\" != inspect ]]; then",
        "    printf 'MUTATION %s\\n' \"$*\" >&2",
        "    return 0",
        "  fi",
        "  local format image capable=0",
        "  if [[ \"$3\" == -f ]]; then format=\"$4\"; image=\"$5\"; else image=\"$3\"; fi",
        "  case \"$image\" in",
        "    legacy-saved) capable=0 ;;",
        "    base-image)",
        "      case \"$PROBE_BASE_STATE\" in capable) capable=1 ;; missing) return 1 ;; stale) ;; esac",
        "      ;;",
        "    *) return 1 ;;",
        "  esac",
        "  [[ \"$3\" == -f ]] || return 0",
        "  case \"$format\" in",
        "    *io.chariox.relay-peer-protocol-version*) (( capable )) && printf '9\\n' || printf '8\\n' ;;",
        "    *io.chariox.runtime-source-revision*) (( capable )) && printf 'runtime-current\\n' || printf 'runtime-stale\\n' ;;",
        "    *io.chariox.selkies-version*) (( capable )) && printf '0.0.0.dev0\\n' || printf '<no value>\\n' ;;",
        "    *io.chariox.selkies-source-revision*) (( capable )) && printf '0123456789012345678901234567890123456789\\n' || printf '<no value>\\n' ;;",
        "    *io.chariox.selkies-source*) (( capable )) && printf 'https://github.com/selkies-project/selkies/commit/0123456789012345678901234567890123456789\\n' || printf '<no value>\\n' ;;",
        "    *io.chariox.selkies-license*) (( capable )) && printf 'MPL-2.0\\n' || printf '<no value>\\n' ;;",
        "    *) printf '<no value>\\n' ;;",
        "  esac",
        "}",
        "SLICE_DISPLAY_BACKEND=selkies",
        "SLICE_SAVED_HOME_ARCHIVE=/etc/hosts",
        "SLICE_HOME_VOLUME=saved-home",
        "SLICE_NAME=saved-state-policy-probe",
        "SLICE_IMAGE=legacy-saved",
        "SLICE_BASE_IMAGE=base-image",
        "SLICE_RELAY_PEER_PROTOCOL_VERSION=9",
        "SLICE_RUNTIME_SOURCE_REVISION=runtime-current",
        "SLICE_BUILD_IMAGE=\"$PROBE_POLICY\"",
        "REPO_ROOT=/nonexistent",
        "SLICE_EXTENSION_DOCKERFILE=",
        runtimeCompatible,
        selkiesCapable,
        basePreflight,
        compatibility,
        buildStandard,
        buildImage,
        "trap 'status=$?; printf \"STATUS=%s\\nIMAGE=%s\\nBASE_STATE=%s\\n\" \"$status\" \"$SLICE_IMAGE\" \"$PROBE_BASE_STATE\"' EXIT",
        "build_image",
      ].join("\n"),
    ],
    {
      env: {
        ...process.env,
        PROBE_POLICY: policy,
        PROBE_BASE_STATE: baseState,
        PROBE_BUILD_RESULT: "capable",
      },
      encoding: "utf8",
    },
  )
}

test("saved state accepts a current image only from authoritative runtime and Selkies labels", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const compatibility = section(source, "require_saved_state_compatibility() {", "build_standard_runtime_image() {")
  const ensure = section(source, "ensure_container() {", "ensure_auth_target_container() {")

  assert.match(compatibility, /io\.chariox\.selkies-version/)
  assert.match(compatibility, /io\.chariox\.selkies-source-revision/)
  assert.match(compatibility, /io\.chariox\.selkies-source/)
  assert.match(compatibility, /io\.chariox\.selkies-license/)
  assert.match(compatibility, /no container or home-volume mutation was attempted/)
  assert.ok(ensure.indexOf("require_saved_state_compatibility") < ensure.indexOf("container_exists"))

  const probe = runSavedStateProbe(source, "compatible")
  const output = `${probe.stdout}${probe.stderr}`
  assert.equal(probe.status, 0, probe.stderr)
  assert.match(output, /STATUS=0/)
  assert.match(output, /IMAGE=saved-current/)
  assert.doesNotMatch(output, /DOCKER_CALL (?!image inspect)/)
})

test("legacy saved state rebases onto a capable runtime while preserving the home archive", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const probe = runSavedStateProbe(source, "legacy")
  const output = `${probe.stdout}${probe.stderr}`

  assert.equal(probe.status, 0, probe.stderr)
  assert.match(output, /STATUS=0/)
  assert.match(output, /IMAGE=current-base/)
  assert.match(output, /RESTORE_STATUS=0/)
  assert.match(output, /saved state migration: restoring \/dev\/null on Selkies-capable runtime image current-base/)
  assert.match(output, /home\/slice state is preserved/)
  assert.match(output, /DOCKER_CALL create .* -v saved-home:\/home-dst current-base sleep infinity/)
  assert.match(output, /DOCKER_CALL cp -L \/dev\/null .*:\/tmp\/home\.tar\.zst/)
})

test("legacy saved state rejects without a capable base before any mutation and explains migration", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const probe = runSavedStateProbe(source, "rejected")
  const output = `${probe.stdout}${probe.stderr}`

  assert.equal(probe.status, 0, probe.stderr)
  assert.match(output, /STATUS=91/)
  assert.match(output, /saved state migration required before selecting Selkies/)
  assert.match(output, /no container or home-volume mutation was attempted/)
  assert.match(output, /CHARIOX_SLICE_DISPLAY_BACKEND=novnc/)
  const nonInspectCalls = output
    .split("\n")
    .filter((line) => line.startsWith("DOCKER_CALL ") && !line.startsWith("DOCKER_CALL image inspect"))
  assert.deepEqual(nonInspectCalls, [])
})

test("saved-state auto and always build policies accept missing and stale capable bases before the compatibility gate", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const buildImage = section(source, "build_image() {", "refresh_saved_state_runtime() {")
  assert.ok(
    buildImage.indexOf("ensure_saved_state_capable_base") < buildImage.indexOf("require_saved_state_compatibility"),
    "Selkies saved-state base preflight must precede the compatibility gate",
  )

  for (const policy of ["auto", "always"]) {
    for (const baseState of ["missing", "stale"]) {
      const probe = runSavedStatePolicyProbe(source, policy, baseState)
      const output = `${probe.stdout}${probe.stderr}`
      assert.equal(probe.status, 0, `${policy}/${baseState}: ${probe.stderr}`)
      assert.match(output, /STATUS=0/)
      assert.match(output, /IMAGE=base-image/)
      assert.match(output, /BASE_STATE=capable/)
      assert.equal((output.match(/DOCKER_BUILD /g) ?? []).length, 1, `${policy}/${baseState} should build once`)
      assert.doesNotMatch(output, /MUTATION /)

      const buildIndex = output.indexOf("DOCKER_BUILD ")
      const labelsAcceptedIndex = output.indexOf("saved state base labels accepted")
      const gateIndex = output.indexOf("saved state compatibility gate")
      assert.ok(buildIndex >= 0)
      assert.ok(labelsAcceptedIndex > buildIndex, `${policy}/${baseState} must re-check labels after build`)
      assert.ok(gateIndex > labelsAcceptedIndex, `${policy}/${baseState} must gate after label acceptance`)
    }
  }
})

test("saved-state never policy rejects missing and stale bases without building or mutating", async () => {
  const source = await readFile(provisionerPath, "utf8")

  for (const baseState of ["missing", "stale"]) {
    const probe = runSavedStatePolicyProbe(source, "never", baseState)
    const output = `${probe.stdout}${probe.stderr}`
    assert.equal(probe.status, 91, `never/${baseState}: ${probe.stderr}`)
    assert.match(output, /STATUS=91/)
    assert.doesNotMatch(output, /DOCKER_BUILD /)
    assert.doesNotMatch(output, /MUTATION /)
    assert.match(output, /saved state compatibility gate: validating image labels before mutation/)
    assert.match(output, /no container or home-volume mutation was attempted/)
  }
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
