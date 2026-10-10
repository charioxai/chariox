import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtemp, mkdir, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

const root = new URL("../", import.meta.url)
const runtime = await readFile(new URL("apps/kernel/slice-linux-docker/docker/start-runtime.sh", root), "utf8")
const start = runtime.match(/start_slice_kernel\(\) \{[\s\S]*?\n\}/)?.[0]
assert.ok(start, "slice runtime has one kernel launch function")
const waitForAuth = runtime.match(/wait_for_kernel_auth_consumption\(\) \{[\s\S]*?\n\}/)?.[0]
assert.ok(waitForAuth, "slice runtime waits for local auth consumption")

const vectors = JSON.parse(await readFile(new URL("fixtures/slice-worker-identity.json", root), "utf8")).cases

async function launch(identity, extraArguments = []) {
  const scratch = await mkdtemp(join(tmpdir(), "chariox-slice-identity-"))
  try {
    await mkdir(join(scratch, "private"))
    const script = [
      "set -Eeuo pipefail",
      "kernel_relay_env=()",
      "KERNEL_LOCAL_AUTH_FILE=\"$ROOT/private/kernel-local-auth.token\"",
      "dd() { printf 'synthetic-local-auth-fixture'; }",
      "sleep() { :; }",
      "wait_for_screen_session() { :; }",
      "screen() { for argument in \"$@\"; do case \"$argument\" in CHARIOX_DAEMON_ID=*|CHARIOX_DAEMON_ALIAS=*|CHARIOX_MACHINE_ID=*) printf '%s\\n' \"$argument\" ;; esac; done; rm \"$KERNEL_LOCAL_AUTH_FILE\"; }",
      waitForAuth,
      start,
      `start_slice_kernel ${extraArguments.join(" ")}`,
    ].join("\n")
    const variables = Object.fromEntries([
      "KERNEL_PORT", "MCP_PORT", "CODEX_PORT_RANGE", "OPENCODE_PORT_RANGE", "PROVIDER_BIND_HOST", "MACHINE_ALIAS", "SLICE_ID", "SLICE_OWNER_KERNEL_ID", "SLICE_OWNER_MACHINE_ID", "SLICE_OWNER_PUBLIC_KEY", "CAPABILITY_ISOLATION_ROOT", "BROWSER_DOWNLOAD_DIR", "BROWSER_UPLOAD_ROOTS", "PROVIDER_HOME",
    ].map(name => [name, "synthetic-fixture"]))
    const result = spawnSync("bash", ["-c", script], {
      encoding: "utf8", timeout: 5_000,
      env: { PATH: process.env.PATH, ...variables, ROOT: scratch, DAEMON_ID: identity.workerKernelRef, DAEMON_ALIAS: `slice:${identity.localName}`, MACHINE_ID: identity.machineId, SLICE_OWNER_KERNEL_ID: identity.ownerKernelId, SLICE_OWNER_MACHINE_ID: identity.machineId },
    })
    assert.equal(result.status, 0, result.stderr)
    return result.stdout.trim().split("\n")
  } finally { await rm(scratch, { recursive: true, force: true }) }
}

test("hosted slice launch registers the exact signed worker subject as canonical daemon ID", async () => {
  for (const identity of vectors) {
    assert.deepEqual(await launch(identity), [
      `CHARIOX_DAEMON_ID=${identity.workerKernelRef}`, `CHARIOX_DAEMON_ALIAS=slice:${identity.localName}`, `CHARIOX_MACHINE_ID=${identity.machineId}`,
    ])
  }
})

test("provider isolation probe uses the same canonical slice identity as the persistent kernel", async () => {
  const identity = vectors[0]
  assert.deepEqual(await launch(identity, ["CHARIOX_ACCEPT_REMOTE_LEASES=0"]), [
    `CHARIOX_DAEMON_ID=${identity.workerKernelRef}`, `CHARIOX_DAEMON_ALIAS=slice:${identity.localName}`, `CHARIOX_MACHINE_ID=${identity.machineId}`,
  ])
})

test("MP-08/MP-11 canonical signed relay admission is coordinated at local protocol 491 and peer protocol 96", async () => {
  const [rust, client, peer] = await Promise.all([
    readFile(new URL("apps/kernel/src/local/api/types.rs", root), "utf8"),
    readFile(new URL("packages/kernel-client/src/kernel-types.ts", root), "utf8"),
    readFile(new URL("apps/kernel/src/transport/relay_peer.rs", root), "utf8"),
  ])
  assert.equal(Number(rust.match(/LOCAL_DAEMON_PROTOCOL_VERSION: u32 = (\d+)/)?.[1]), 491)
  assert.equal(Number(client.match(/LOCAL_DAEMON_PROTOCOL_VERSION = (\d+)/)?.[1]), 491)
  assert.equal(Number(peer.match(/RELAY_PEER_PROTOCOL_VERSION: u32 = (\d+)/)?.[1]), 101)
})
