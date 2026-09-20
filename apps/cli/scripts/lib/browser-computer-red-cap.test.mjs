import assert from "node:assert/strict"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  AUTHORITATIVE_MAIN_SHA,
  BROWSER_COMPUTER_RED_CAP_EVIDENCE_SCHEMA,
  RED_CAP_FAULTS,
  RED_CAP_MAX_GROWTH_BYTES,
  RED_CAP_MIN_FREE_BYTES,
  artifactBytes,
  assertSafeExternalPath,
  buildFaultCommand,
  buildRedCapCommandPlan,
  buildRedCapPorts,
  classifyRedCapAssertion,
  cleanupRedCapRun,
  createProcessRegistry,
  evaluateRedCapResourceCeilings,
  makeRedCapAssertion,
  redCapCaseIds,
  redCapManifestStatus,
  resolveRedCapProfile,
  spawnBounded,
  validateRedCapEvidenceManifest,
} from "./browser-computer-red-cap.mjs"
import { browserComputerFunctionalCases } from "./browser-computer-functional-contract.mjs"

const REPO_ROOT = path.resolve(new URL("../../../..", import.meta.url).pathname)

test("red-cap profiles and command construction are bounded and use the pre-cutover stack", () => {
  assert.equal(resolveRedCapProfile("mac").id, "mac-docker-desktop")
  assert.equal(resolveRedCapProfile("linux-docker").browserControl, "one-shot-cdp")
  const plan = buildRedCapCommandPlan({
    profile: "linux-docker",
    repoRoot: "/work/chariox",
    runRoot: "/tmp/chariox-m0-red-cap-test",
    runId: "m0-test",
    ports: buildRedCapPorts({ novnc: 16080, vnc: 15900, kernel: 16119, relay: 16130, mcp: 16120 }),
    containerName: "chariox-m0-red-cap-test",
  })
  assert.deepEqual(plan.provision.args, [
    "/work/chariox/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
    "provision",
  ])
  assert.equal(plan.provision.env.CHARIOX_SLICE_BUILD_IMAGE, "never")
  assert.equal(plan.provision.env.CHARIOX_SLICE_START_RUNTIME, "0")
  assert.deepEqual(plan.cdp(["status"]).args.slice(-3), ["node", "/opt/chariox-slice/browser-cdp.mjs", "status"])
  assert.deepEqual(plan.screen(["status"]).args.slice(-2), ["/opt/chariox-slice/slice-screen.sh", "status"])
  assert.equal(plan.containerName, "chariox-m0-red-cap-test")
})

test("fault controls have deterministic order, exact targets, and bounded commands", () => {
  const plan = buildRedCapCommandPlan({
    profile: "linux-docker",
    repoRoot: "/work/chariox",
    runRoot: "/tmp/chariox-m0-red-cap-test",
    runId: "m0-test",
    containerName: "chariox-m0-red-cap-test",
  })
  const offsets = RED_CAP_FAULTS.map((fault) => fault.offsetMs)
  assert.deepEqual(offsets, [...offsets].sort((a, b) => a - b))
  for (const fault of RED_CAP_FAULTS) {
    const command = buildFaultCommand(plan, fault.id, { staleUrl: "http://127.0.0.1:16080/vnc.html" })
    assert.equal(command.id, fault.id)
    assert.ok(command.args.length > 0)
    if (!command.args.includes("chariox-m0-red-cap-test")) assert.match(command.args.at(-1), /127\.0\.0\.1:16080/)
  }
  assert.throws(() => buildFaultCommand(plan, "unknown", {}), /unknown red-cap fault/)
})

test("red-cap classification keeps expected red distinct from unexpected red and green", () => {
  assert.equal(classifyRedCapAssertion({ status: "failed", expectedOutcome: "fail", evidence: "missing M1.1 authority" }).classification, "red-expected")
  assert.equal(classifyRedCapAssertion({ status: "failed", expectedOutcome: "pass", evidence: "unexpected failure" }).classification, "red-unexpected")
  assert.equal(classifyRedCapAssertion({ status: "passed", expectedOutcome: "fail", evidence: "implementation now exists" }).classification, "green-implementation")
  assert.equal(classifyRedCapAssertion({ status: "passed", expectedOutcome: "pass", evidence: "verified" }).classification, "green")
  assert.equal(classifyRedCapAssertion({ status: "blocked", expectedOutcome: "pass", evidence: "Docker unavailable" }).classification, "blocked")
})

test("the red-cap reuses the canonical functional catalog without duplicating case ids", () => {
  assert.deepEqual(redCapCaseIds(), browserComputerFunctionalCases().map((item) => item.id))
  assert.equal(new Set(redCapCaseIds()).size, redCapCaseIds().length)
})

test("evidence validation requires exact case coverage, monotonic time, commands, resources, and cleanup", async () => {
  const artifactRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-m0-red-cap-evidence-"))
  try {
    const startedAt = "2026-09-20T00:00:00.000Z"
    const completedAt = "2026-09-20T00:00:01.000Z"
    const assertions = redCapCaseIds().flatMap((caseId) => browserComputerFunctionalCases().find((item) => item.id === caseId).assertions.map((id) => makeRedCapAssertion({
      id,
      caseId,
      status: "passed",
      expectedOutcome: "pass",
      evidence: "focused evidence fixture",
    })))
    const manifest = {
      schema: BROWSER_COMPUTER_RED_CAP_EVIDENCE_SCHEMA,
      runId: "m0-evidence-test",
      testedSha: AUTHORITATIVE_MAIN_SHA,
      originMainSha: AUTHORITATIVE_MAIN_SHA,
      startedAt,
      completedAt,
      startedMonoNs: 100,
      completedMonoNs: 1_000_000_100,
      durationMs: 1_000,
      status: "passed",
      ok: true,
      profile: { id: "linux-docker", platform: "linux", topology: "local-docker" },
      stack: { display: "novnc", browserControl: "one-shot-cdp" },
      artifactRoot,
      caseIds: redCapCaseIds(),
      commands: [{ label: "fixture", command: "node", args: ["--version"], pid: 123, timeoutMs: 5_000, code: 0 }],
      assertions,
      resourceSamples: [{ capturedAt: startedAt, disk: { availableBytes: RED_CAP_MIN_FREE_BYTES } }],
      artifacts: [{ path: path.join(artifactRoot, "screenshot.png"), kind: "screenshot" }],
      cleanup: { ok: true, violations: [] },
    }
    assert.equal(validateRedCapEvidenceManifest(manifest, { repoRoot: REPO_ROOT }), manifest)
    const invalid = structuredClone(manifest)
    invalid.assertions[0].classification = "red-unexpected"
    assert.throws(() => validateRedCapEvidenceManifest(invalid, { repoRoot: REPO_ROOT }), /classification does not match|known expected failures|red-unexpected/)
  } finally {
    await rm(artifactRoot, { recursive: true, force: true })
  }
})

test("owned cleanup removes only the disposable root and exact owned Docker names", async () => {
  const tempRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-m0-red-cap-cleanup-"))
  const calls = []
  try {
    await writeFile(path.join(tempRoot, "ownership.json"), JSON.stringify({ tempRoot }), { mode: 0o600 })
    const plan = buildRedCapCommandPlan({
      profile: "linux-docker",
      repoRoot: "/work/chariox",
      runRoot: tempRoot,
      runId: "m0-cleanup",
      containerName: "chariox-m0-red-cap-cleanup",
      volumeName: "chariox-m0-red-cap-cleanup-home",
    })
    const result = await cleanupRedCapRun({
      paths: { tempRoot, repoRoot: "/work/chariox" },
      plan,
      registry: createProcessRegistry(),
      createdContainer: true,
      createdVolume: true,
      runCommand: async (command, args, options) => {
        calls.push({ command, args, options })
        return { code: 0, signal: null, stdout: "", stderr: "" }
      },
    })
    assert.equal(result.ok, true)
    assert.deepEqual(calls.map((call) => call.args), [
      ["rm", "--force", "chariox-m0-red-cap-cleanup"],
      ["volume", "rm", "--force", "chariox-m0-red-cap-cleanup-home"],
    ])
    await assert.rejects(() => readFile(path.join(tempRoot, "ownership.json")))
  } finally {
    await rm(tempRoot, { recursive: true, force: true })
  }
})

test("bounded child processes expose exact PIDs and terminate on timeout", async () => {
  const registry = createProcessRegistry()
  const result = await spawnBounded(process.execPath, ["-e", "setTimeout(() => {}, 10_000)"], {
    timeoutMs: 80,
    label: "timeout-fixture",
    registry,
  })
  assert.equal(typeof result.pid, "number")
  assert.equal(result.timedOut, true)
  assert.ok(result.signal || result.code !== 0)
  assert.equal(registry.active.size, 0)
})

test("resource ceilings fail closed below free-space and growth limits", () => {
  const passing = evaluateRedCapResourceCeilings({
    before: { disk: { availableBytes: RED_CAP_MIN_FREE_BYTES + 20 * 1024 * 1024 } },
    after: { disk: { availableBytes: RED_CAP_MIN_FREE_BYTES + 10 * 1024 * 1024 } },
    artifactBytes: 1 * 1024 * 1024,
  })
  assert.equal(passing.ok, true)
  const failing = evaluateRedCapResourceCeilings({
    before: { disk: { availableBytes: RED_CAP_MIN_FREE_BYTES } },
    after: { disk: { availableBytes: RED_CAP_MIN_FREE_BYTES - 1 } },
    artifactBytes: RED_CAP_MAX_GROWTH_BYTES + 1,
  })
  assert.equal(failing.ok, false)
  assert.equal(failing.violations.length, 2)
})

test("unsafe paths are rejected and evidence growth can be measured", async () => {
  assert.throws(() => assertSafeExternalPath("/", { repoRoot: "/work/chariox", kind: "root" }), /filesystem root/)
  assert.throws(() => assertSafeExternalPath("/work/chariox/artifacts", { repoRoot: "/work/chariox", kind: "repo path" }), /outside the repository/)
  assert.throws(() => assertSafeExternalPath("/tmp/not-disposable", { repoRoot: "/work/chariox", kind: "temp", disposable: true }), /disposable/)
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-m0-red-cap-growth-"))
  try {
    await writeFile(path.join(root, "evidence.json"), "12345")
    assert.equal(await artifactBytes(root), 5)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("manifest status is explicit when expected red remains", () => {
  const expected = makeRedCapAssertion({ id: "x", caseId: "shared.takeover", status: "failed", expectedOutcome: "fail", evidence: "known gap" })
  const unexpected = makeRedCapAssertion({ id: "y", caseId: "browser.form", status: "failed", expectedOutcome: "pass", evidence: "regression" })
  assert.equal(redCapManifestStatus([expected], true), "expected-red")
  assert.equal(redCapManifestStatus([unexpected], true), "failed")
  assert.equal(redCapManifestStatus([expected], false), "blocked")
})
