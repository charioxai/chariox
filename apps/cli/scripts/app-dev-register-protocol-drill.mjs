#!/usr/bin/env node
// MP-08 / MP-10 / MP-11: focused protocol 644 drill against a real kernel.
// Build the CLI, then supply a frozen protocol-567 CLI transport bundle exporting
// LocalIpcClient and LOCAL_DAEMON_PROTOCOL_VERSION. Use an existing owner's Room:
// node scripts/app-dev-register-protocol-drill.mjs --kernel-url ws://127.0.0.1:44240/kernel \
//   --session-id ROOM --legacy-client /outside/repo/legacy-client.cjs --evidence /outside/repo/result.json
// Missing uploads deliberately stop at normal App admission: this proves wire
// compatibility, not App execution or the S1-02/S1-03 live acceptance scenarios.
import assert from "node:assert/strict"
import { createHash, randomUUID } from "node:crypto"
import { readFile, writeFile } from "node:fs/promises"
import { resolve } from "node:path"
import { pathToFileURL } from "node:url"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../dist/ipc.js"
import { loadLocalKernelPresences, localKernelEndpoint } from "../dist/local-kernel-presence.js"
import { beginAppInstallRequest, beginAppUpdateRequest } from "../../../packages/kernel-client/dist/ipc-app-requests.js"

const args = process.argv.slice(2)
const option = name => { const index = args.indexOf(`--${name}`); return index >= 0 ? args[index + 1] : undefined }
const endpoint = option("kernel-url")
const session = option("session-id")
const legacyPath = option("legacy-client")
assert.ok(endpoint && session && legacyPath, "--kernel-url, --session-id and --legacy-client are required")
const presence = loadLocalKernelPresences().find(value => localKernelEndpoint(value) === endpoint)
assert.ok(presence?.protocolVersion >= 644, "the live target must advertise protocol 644 or newer")
const imported = await import(pathToFileURL(resolve(legacyPath)).href)
const legacy = imported.default ?? imported
assert.equal(legacy.LOCAL_DAEMON_PROTOCOL_VERSION, 567, "use a frozen protocol-567 client")
const old = new legacy.LocalIpcClient(endpoint)
const current = new LocalIpcClient(endpoint)
const steps = []
async function decoded(client, request, update) {
  const reply = await client.send(request)
  if (update) {
    assert.equal(reply.AppRequestFailed?.code, "not_found", "update must reach normal installation lookup")
    return { code: reply.AppRequestFailed.code }
  }
  let operation = reply.AppInstallOperationStatus?.operation
  assert.ok(operation, "install must decode and reserve an operation")
  const initialPhase = operation.phase
  const deadline = Date.now() + 10_000
  while (operation.phase !== "failed" && Date.now() < deadline) {
    await delay(100)
    const status = await client.send({ GetAppInstallOperation: { request_id: operation.request_id } })
    operation = status.AppInstallOperationStatus?.operation
    assert.ok(operation, "reserved install must remain readable")
  }
  if (operation.phase !== "failed") {
    await client.send({ CancelAppInstallOperation: { request_id: operation.request_id } })
    assert.fail("missing-upload install did not settle within ten seconds")
  }
  assert.equal(operation.failure, "app_install_upload_missing_or_expired")
  return { initial_phase: initialPhase, phase: operation.phase, failure: operation.failure }
}
try {
  const list = await old.send({ ListAppInstallations: { after: null, limit: 50 } })
  assert.ok(Array.isArray(list.AppInstallationsListed?.installations))
  steps.push({ name: "567 client lists installations on the new kernel", result: "PASS" })
  const options = () => ({ sessionId: session, requestId: `protocol-drill-${randomUUID()}`,
    uploadHandle: `upload_${"a".repeat(64)}`, expectedPackageDigest: `sha256:${"b".repeat(64)}` })
  // Frozen 567 shapes deliberately omit the newly introduced source field.
  for (const kind of ["BeginAppInstall", "BeginAppUpdate"]) {
    const input = options()
    const payload = { session_id: session, request_id: input.requestId,
      upload_handle: input.uploadHandle, expected_package_digest: input.expectedPackageDigest,
      ...(kind === "BeginAppUpdate" ? { installation_id: "missing", expected_generation: "1" } : {}) }
    const result = await decoded(old, { [kind]: payload }, kind === "BeginAppUpdate")
    steps.push({ name: `567 ${kind} without source`, result: "PASS", ...result })
  }
  for (const source of ["dev_push", "register"]) {
    for (const kind of ["install", "update"]) {
      const input = { ...options(), source }
      const request = kind === "install" ? beginAppInstallRequest(input)
        : beginAppUpdateRequest({ ...input, installationId: "missing", expectedGeneration: "1" })
      const result = await decoded(current, request, kind === "update")
      steps.push({ name: `${source} ${kind}`, result: "PASS", ...result })
    }
  }
  const evidence = { scope: "MP-08 / MP-10 / MP-11 protocol compatibility only", date: new Date().toISOString(),
    kernel_protocol: presence.protocolVersion, legacy_protocol: legacy.LOCAL_DAEMON_PROTOCOL_VERSION,
    legacy_client_sha256: createHash("sha256").update(await readFile(resolve(legacyPath))).digest("hex"), steps }
  if (option("evidence")) await writeFile(option("evidence"), `${JSON.stringify(evidence, null, 2)}\n`)
  console.log(JSON.stringify(evidence))
  console.log("APP_DEV_REGISTER_PROTOCOL_DRILL_PASS")
} finally {
  await Promise.all([old.close(), current.close()])
}
