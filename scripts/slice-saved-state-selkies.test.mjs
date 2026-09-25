import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { readFile } from "node:fs/promises"
import test from "node:test"
import { fileURLToPath } from "node:url"

const provisionerPath = fileURLToPath(new URL("../apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh", import.meta.url))

function section(source, startMarker, endMarker) {
  const start = source.indexOf(startMarker)
  const end = source.indexOf(endMarker, start + startMarker.length)
  assert.notEqual(start, -1, `missing section start: ${startMarker}`)
  assert.notEqual(end, -1, `missing section end: ${endMarker}`)
  return source.slice(start, end)
}

function runProbe(lines, environment = {}) {
  return spawnSync("bash", ["-c", ["set -Eeuo pipefail", ...lines].join("\n")], {
    encoding: "utf8",
    env: { ...process.env, ...environment },
  })
}

function runCapabilityProbe(source, currentState, baseState) {
  const imageFunctions = section(source, "image_runtime_compatible() {", "docker_target_arch() {")
  const compatibility = section(source, "require_saved_state_compatibility() {", "docker_target_arch() {")
  return runProbe([
    "log() { printf '[probe] %s\\n' \"$*\" >&2; }",
    "fail() { printf 'FAIL %s\\n' \"$*\" >&2; return 91; }",
    "docker() {",
    "  printf 'DOCKER_CALL %s\\n' \"$*\" >&2",
    "  if [[ \"$1\" != image || \"$2\" != inspect ]]; then printf 'MUTATION %s\\n' \"$*\" >&2; return 42; fi",
    "  local format='' image='' state=''",
    "  if [[ \"$3\" == -f ]]; then format=\"$4\"; image=\"$5\"; else image=\"$3\"; fi",
    "  case \"$image\" in saved-image) state=\"$PROBE_CURRENT_STATE\" ;; base-image) state=\"$PROBE_BASE_STATE\" ;; *) return 1 ;; esac",
    "  [[ \"$3\" == -f ]] || [[ \"$state\" != missing ]] || return 1",
    "  [[ \"$3\" == -f ]] || return 0",
    "  [[ \"$state\" == capable || \"$state\" == runtime-only ]] || { printf '<no value>\\n'; return 1; }",
    "  case \"$format\" in",
    "    *relay-peer-protocol-version*) [[ \"$state\" == capable || \"$state\" == runtime-only ]] && printf '%s\\n' current ;;",
    "    *runtime-source-revision*) [[ \"$state\" == capable || \"$state\" == runtime-only ]] && printf '%s\\n' runtime-current ;;",
    "    *selkies-version*) [[ \"$state\" == capable ]] && printf '%s\\n' 0.0.0.dev0 || printf '<no value>\\n' ;;",
    "    *selkies-source-revision*) [[ \"$state\" == capable ]] && printf '%s\\n' 0123456789012345678901234567890123456789 || printf '<no value>\\n' ;;",
    "    *selkies-source*) [[ \"$state\" == capable ]] && printf '%s\\n' https://github.com/selkies-project/selkies/commit/0123456789012345678901234567890123456789 || printf '<no value>\\n' ;;",
    "    *selkies-license*) [[ \"$state\" == capable ]] && printf '%s\\n' MPL-2.0 || printf '<no value>\\n' ;;",
    "    *) printf '<no value>\\n' ;;",
    "  esac",
    "}",
    "SLICE_SAVED_HOME_ARCHIVE=/etc/hosts",
    "SLICE_VIEWER_BACKEND=selkies",
    "SLICE_IMAGE=saved-image",
    "SLICE_BASE_IMAGE=base-image",
    "SLICE_RELAY_PEER_PROTOCOL_VERSION=current",
    "SLICE_RUNTIME_SOURCE_REVISION=runtime-current",
    imageFunctions,
    compatibility,
    "status=0",
    "if require_saved_state_compatibility; then status=0; else status=$?; fi",
    "printf 'STATUS=%s\\nIMAGE=%s\\n' \"$status\" \"$SLICE_IMAGE\"",
  ], { PROBE_CURRENT_STATE: currentState, PROBE_BASE_STATE: baseState })
}

function runHomeVolumeProbe(source, volumeState, failRestore = false) {
  const volumeFunctions = section(source, "volume_inspect_reports_not_found() {", "machine_id_hex() {")
  return runProbe([
    "log() { printf '[probe] %s\\n' \"$*\" >&2; }",
    "fail() { printf 'FAIL %s\\n' \"$*\" >&2; return 91; }",
    "run_with_timeout() { local seconds=\"$1\"; shift; \"$@\"; }",
    "hash_stdin() { printf '%064d\\n' 0 | tr 0 a; }",
    "docker() {",
    "  printf 'DOCKER_CALL %s\\n' \"$*\" >&2",
    "  case \"$1 $2\" in",
    "    'volume inspect')",
    "      if [[ \"$3\" == -f ]]; then",
    "        if [[ \"$4\" == *archive-sha256* ]]; then printf '%s\\n' \"$PROBE_ARCHIVE_LABEL\"; else printf '%s\\n' \"$PROBE_TOKEN_LABEL\"; fi",
    "      elif [[ \"$PROBE_VOLUME_STATE\" == existing ]]; then return 0",
    "      else printf 'Error: no such volume: saved-home\\n' >&2; return 1; fi",
    "      ;;",
    "    'volume create')",
    "      PROBE_VOLUME_STATE=existing",
    "      local previous='' argument",
    "      for argument in \"$@\"; do",
    "        if [[ \"$previous\" == --label ]]; then",
    "          case \"$argument\" in io.chariox.saved-home.archive-sha256=*) PROBE_ARCHIVE_LABEL=\"${argument#*=}\" ;; io.chariox.saved-home.initialization-token=*) PROBE_TOKEN_LABEL=\"${argument#*=}\" ;; esac",
    "        fi",
    "        previous=\"$argument\"",
    "      done",
    "      ;;",
    "    'volume rm') PROBE_VOLUME_STATE=missing ;;",
    "    'rm -f '* ) ;;",
    "    'create '* ) [[ \"$PROBE_FAIL_RESTORE\" == 1 ]] && return 42 ;;",
    "    'start '* ) [[ \"$PROBE_FAIL_RESTORE\" == 1 ]] && return 42 ;;",
    "    'cp -L') [[ \"$PROBE_FAIL_RESTORE\" == 1 ]] && return 42 ;;",
    "    'exec -u') [[ \"$PROBE_FAIL_RESTORE\" == 1 ]] && return 42 ;;",
    "    *) printf 'MUTATION %s\\n' \"$*\" >&2; return 42 ;;",
    "  esac",
    "}",
    "SLICE_SAVED_HOME_ARCHIVE=/etc/hosts",
    "SLICE_HOME_VOLUME=saved-home",
    "SLICE_NAME=saved-state-probe",
    "SLICE_IMAGE=current-image",
    "PROBE_ARCHIVE_LABEL=''",
    "PROBE_TOKEN_LABEL=''",
    volumeFunctions,
    "status=0",
    "if prepare_home_volume; then status=0; else status=$?; fi",
    "printf 'STATUS=%s\\nVOLUME_STATE=%s\\nARCHIVE_LABEL=%s\\nTOKEN_LABEL=%s\\n' \"$status\" \"$PROBE_VOLUME_STATE\" \"$PROBE_ARCHIVE_LABEL\" \"$PROBE_TOKEN_LABEL\"",
  ], { PROBE_VOLUME_STATE: volumeState, PROBE_FAIL_RESTORE: failRestore ? "1" : "0" })
}

test("saved-state capability is checked before a Selkies volume mutation", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const dockerfile = await readFile(new URL("../apps/kernel/slice-linux-docker/docker/Dockerfile", import.meta.url), "utf8")
  assert.match(source, /SLICE_VIEWER_BACKEND="\$\{CHARIOX_SLICE_VIEWER_BACKEND:-selkies\}"/)
  assert.match(dockerfile, /LABEL io\.chariox\.selkies-source-revision/)
  const buildImage = section(source, "build_image() {", "refresh_saved_state_runtime() {")
  assert.ok(buildImage.indexOf("ensure_saved_state_capable_base") < buildImage.indexOf("require_saved_state_compatibility"))
  const ensure = section(source, "ensure_container() {", "exec_slice_with_timeout() {")
  assert.ok(ensure.indexOf("require_saved_state_compatibility") < ensure.indexOf("container_exists"))
  assert.match(ensure, /prepare_home_volume/)
})

test("compatible Selkies saved state remains on the selected image", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const probe = runCapabilityProbe(source, "capable", "missing")
  const output = `${probe.stdout}${probe.stderr}`
  assert.equal(probe.status, 0, probe.stderr)
  assert.match(output, /STATUS=0/)
  assert.match(output, /IMAGE=saved-image/)
  assert.doesNotMatch(output, /MUTATION/)
})

test("legacy Selkies saved state falls back to a capable base without mutating first", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const probe = runCapabilityProbe(source, "runtime-only", "capable")
  const output = `${probe.stdout}${probe.stderr}`
  assert.equal(probe.status, 0, probe.stderr)
  assert.match(output, /STATUS=0/)
  assert.match(output, /IMAGE=base-image/)
  assert.doesNotMatch(output, /MUTATION/)
})

test("Selkies saved state fails closed when no capable image exists", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const probe = runCapabilityProbe(source, "runtime-only", "runtime-only")
  const output = `${probe.stdout}${probe.stderr}`
  assert.equal(probe.status, 0, probe.stderr)
  assert.match(output, /STATUS=91/)
  assert.match(output, /no runtime-compatible image with authoritative Selkies capability labels/)
  assert.match(output, /no container or home-volume mutation was attempted/)
  assert.doesNotMatch(output, /MUTATION/)
})

test("existing saved-state volumes are preserved instead of replayed", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const probe = runHomeVolumeProbe(source, "existing")
  const output = `${probe.stdout}${probe.stderr}`
  assert.equal(probe.status, 0, probe.stderr)
  assert.match(output, /STATUS=0/)
  assert.match(output, /preserving existing home volume saved-home/)
  assert.doesNotMatch(output, /restoring saved home archive/)
})

test("new saved-state volumes are labeled and cleaned up after restore failure", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const probe = runHomeVolumeProbe(source, "missing", true)
  const output = `${probe.stdout}${probe.stderr}`
  assert.equal(probe.status, 0, probe.stderr)
  assert.match(output, /STATUS=91/)
  assert.match(output, /DOCKER_CALL volume create --label io\.chariox\.saved-home\.archive-sha256=/)
  assert.match(output, /DOCKER_CALL volume rm saved-home/)
})

test("slice screen diagnostics redact JSON-quoted secret keys", async () => {
  const source = await readFile(provisionerPath, "utf8")
  const diagnostics = section(source, "slice_screen_diagnostics() {", "run_required_phase() {")
  const probe = runProbe([
    "log() { printf '%s\\n' \"$*\" >&2; }",
    "run_with_timeout() { printf '%s\\n' \"$PROBE_DIAGNOSTICS\"; }",
    "SLICE_NAME=diagnostic-probe",
    diagnostics,
    "slice_screen_diagnostics",
  ], {
    PROBE_DIAGNOSTICS: [
      '{"relay_token": "CANARY_JSON_TOKEN"}',
      '{"password":"CANARY_JSON_PASSWORD"}',
      "ordinary diagnostic",
    ].join("\n"),
  })
  assert.equal(probe.status, 0, probe.stderr)
  assert.doesNotMatch(probe.stderr, /CANARY_JSON_/)
  assert.match(probe.stderr, /\[REDACTED\]/)
  assert.match(probe.stderr, /ordinary diagnostic/)
})
