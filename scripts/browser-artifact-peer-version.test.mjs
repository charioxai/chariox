// MP-08/MP-10/MP-11: run the production image-preflight predicate with synthetic labels.
import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { spawnSync } from "node:child_process"
import test from "node:test"

const provisioner = await readFile(new URL("../apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh", import.meta.url), "utf8")
const peer = await readFile(new URL("../apps/kernel/src/transport/relay_peer.rs", import.meta.url), "utf8")
const version = Number(peer.match(/RELAY_PEER_PROTOCOL_VERSION: u32 = (\d+)/)?.[1])
const compatible = provisioner.match(/image_runtime_compatible\(\) \{[\s\S]*?\n\}/)?.[0]
assert.ok(compatible)

test("MP-08/MP-10/MP-11 peer73 preflight rejects v70 and accepts only matching v73 image lineage", () => {
  for (const [imageVersion, imageRevision, expected] of [
    [70, "fixture-current", false], [73, "fixture-current", true],
    [73, "fixture-stale", false], ["<no value>", "fixture-current", false],
  ]) {
    const result = spawnSync("bash", ["-c", `
      set -eu
      docker() {
        case "$*" in
          *relay-peer-protocol-version*) printf '%s' "$FIXTURE_PEER" ;;
          *runtime-source-revision*) printf '%s' "$FIXTURE_REVISION" ;;
          *) return 99 ;;
        esac
      }
      ${compatible}
      image_runtime_compatible fixture-image
    `], { encoding: "utf8", timeout: 5000, env: {
      PATH: process.env.PATH, SLICE_RELAY_PEER_PROTOCOL_VERSION: String(version),
      SLICE_RUNTIME_SOURCE_REVISION: "fixture-current", FIXTURE_PEER: String(imageVersion), FIXTURE_REVISION: imageRevision,
    } })
    assert.equal(result.status === 0, expected, `peer=${imageVersion}, source=${imageRevision}: ${result.stderr}`)
  }
  assert.equal(version, 73)
})
