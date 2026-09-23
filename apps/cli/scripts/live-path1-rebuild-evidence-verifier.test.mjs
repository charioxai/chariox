import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import {
  EVIDENCE_SCHEMA,
  RebuildEvidenceError,
  verifyEvidenceArtifactFiles,
  verifyPath1RebuildEvidence,
} from "./live-path1-rebuild-evidence-verifier.mjs"

const times = Object.freeze({
  reviewed: "2026-09-20T10:00:00.000Z",
  before: "2026-09-21T10:00:00.000Z",
  request: "2026-09-21T10:10:00.000Z",
  complete: "2026-09-21T10:20:00.000Z",
  after: "2026-09-21T10:30:00.000Z",
  cleanup: "2026-09-21T10:40:00.000Z",
})

const oldIds = Object.freeze({
  bootId: "old-boot-identity-001",
  machineId: "old-machine-identity-001",
  enrollmentId: "old-enrollment-identity-001",
  relayRegistrationId: "old-relay-registration-001",
  runtimeMachineId: "old-runtime-machine-001",
})

function observation(id, authority, operation, capturedAt) {
  const bytes = `captured:${id}`
  return {
    id,
    authority,
    operation,
    capturedAt,
    evidenceRef: `file:/retained-path1-captures/${id}.json`,
    evidenceSha256: `sha256:${createHash("sha256").update(bytes).digest("hex")}`,
    exitCode: 0,
  }
}

function common(kind, capturedAt, observations, fields) {
  return {
    schema: EVIDENCE_SCHEMA,
    kind,
    campaignId: "path1-rebuild-campaign-20260923-01",
    capturedAt,
    observations,
    ...fields,
  }
}

function validEvidence() {
  const reviewedCommit = "1".repeat(40)
  const releaseDigest = `sha256:${"a".repeat(64)}`
  const sourceTree = "2".repeat(40)
  const approvedImageId = "hetzner-image-ubuntu-2604-approved"
  const allocation = {
    provider: "hetzner",
    projectId: "project-481",
    serverId: "server-9031",
    serverType: "cx33",
    datacenter: "fsn1-dc14",
    imageId: "previous-image-2204",
  }
  const reviewed = common("reviewed", times.reviewed, [
    observation("reviewed-commit", "git-command", "git rev-parse HEAD", times.reviewed),
    observation("reviewed-release", "release-verifier", "verify-image-release", times.reviewed),
    observation("approved-image", "image-approval", "record clean base image approval", times.reviewed),
  ], {
    reviewedCommit,
    approvedImage: {
      imageId: approvedImageId,
      name: "Ubuntu 26.04 clean system image",
      type: "system-image",
      approved: true,
      cleanBase: true,
      approvalRef: "file:/reviewed/path1-image-approval.json",
    },
    release: {
      digest: releaseDigest,
      sourceCommit: reviewedCommit,
      sourceTree,
      signatureVerified: true,
      manifestDigestVerified: true,
      kernelArtifactVerified: true,
    },
  })
  reviewed.observations[2].evidenceRef = reviewed.approvedImage.approvalRef

  const before = common("before", times.before, [
    observation("before-provider", "hetzner-api", "get server before destructive rebuild", times.before),
    observation("before-host", "host-command", "capture systemd processes state and machine identity", times.before),
    observation("before-cloud", "cloud-api", "capture enrollment and runtime control rows", times.before),
    observation("before-relay", "relay-api", "capture relay registrations and heartbeats", times.before),
  ], {
    allocation,
    identity: {
      ...oldIds,
      release: {
        digest: `sha256:${"b".repeat(64)}`,
        sourceCommit: "3".repeat(40),
        sourceTree: "4".repeat(40),
        instanceId: "old-release-device-inode-1001",
      },
      serviceInstances: [{ unit: "chariox-managed-bootstrap.service", invocationId: "old-service-invocation-01" }],
      processes: ["old-boot-identity-001:pid-81:start-442"],
      stateInstances: [
        { path: "/var/lib/chariox", instanceId: "old-state-device-inode-2001" },
        { path: "/home/chariox", instanceId: "old-home-device-inode-2002" },
      ],
      cloudRows: [
        { rowId: "cloud-enrollment-row-01", identity: oldIds.enrollmentId },
        { rowId: "cloud-runtime-row-01", identity: oldIds.runtimeMachineId },
      ],
      heartbeats: [
        { heartbeatId: "relay-registration-heartbeat-01", identity: oldIds.relayRegistrationId },
        { heartbeatId: "relay-runtime-heartbeat-01", identity: oldIds.runtimeMachineId },
      ],
    },
  })

  const rebuild = common("rebuild", times.complete, [
    observation("rebuild-request", "hetzner-api", "server rebuild request", times.request),
    observation("rebuild-completion", "hetzner-api", "server rebuild completion", times.complete),
  ], {
    allocation,
    request: {
      requestId: "hcloud-request-7319",
      actionId: "hcloud-action-8120",
      serverId: allocation.serverId,
      sourceImageId: allocation.imageId,
      targetImageId: approvedImageId,
      requestedAt: times.request,
      destructive: true,
    },
    completion: {
      requestId: "hcloud-request-7319",
      actionId: "hcloud-action-8120",
      serverId: allocation.serverId,
      imageId: approvedImageId,
      status: "success",
      completedAt: times.complete,
    },
  })

  const newIdentity = {
    bootId: "new-boot-identity-002",
    machineId: "new-machine-identity-002",
    enrollmentId: "new-enrollment-identity-002",
    relayRegistrationId: "new-relay-registration-002",
    runtimeMachineId: "new-runtime-machine-002",
    release: {
      digest: releaseDigest,
      sourceCommit: reviewedCommit,
      sourceTree,
      instanceId: "new-release-device-inode-3001",
      signatureVerified: true,
      manifestDigestVerified: true,
      kernelArtifactVerified: true,
    },
    serviceInstances: [{ unit: "chariox-managed-bootstrap.service", invocationId: "new-service-invocation-02" }],
    processes: ["new-boot-identity-002:pid-81:start-18"],
    stateInstances: [
      { path: "/var/lib/chariox", instanceId: "new-state-device-inode-4001" },
      { path: "/home/chariox", instanceId: "new-home-device-inode-4002" },
    ],
  }
  const afterObservations = [
    observation("after-provider", "hetzner-api", "get server after rebuild", times.after),
    observation("after-host", "host-command", "capture new boot systemd processes and state", times.after),
    observation("after-cloud", "cloud-api", "capture new enrollment and control rows", times.after),
    observation("after-relay", "relay-api", "capture new relay registration and heartbeat state", times.after),
    observation("after-release", "release-verifier", "verify-image-release", times.after),
  ]
  const after = common("after", times.after, afterObservations, {
    allocation: { ...allocation, imageId: approvedImageId },
    identity: newIdentity,
    absence: {
      identities: [
        { kind: "bootId", value: oldIds.bootId, present: false, observationId: "after-host" },
        { kind: "machineId", value: oldIds.machineId, present: false, observationId: "after-host" },
        { kind: "enrollmentId", value: oldIds.enrollmentId, present: false, observationId: "after-cloud" },
        { kind: "relayRegistrationId", value: oldIds.relayRegistrationId, present: false, observationId: "after-relay" },
        { kind: "runtimeMachineId", value: oldIds.runtimeMachineId, present: false, observationId: "after-cloud" },
      ],
      services: [{ invocationId: "old-service-invocation-01", present: false, observationId: "after-host" }],
      processes: [{ processId: "old-boot-identity-001:pid-81:start-442", present: false, observationId: "after-host" }],
      stateInstances: [
        { instanceId: "old-state-device-inode-2001", present: false, observationId: "after-host" },
        { instanceId: "old-home-device-inode-2002", present: false, observationId: "after-host" },
      ],
      releaseInstances: [{ instanceId: "old-release-device-inode-1001", present: false, observationId: "after-host" }],
      cloudRows: [
        { rowId: "cloud-enrollment-row-01", present: false, observationId: "after-cloud" },
        { rowId: "cloud-runtime-row-01", present: false, observationId: "after-cloud" },
      ],
      heartbeats: [
        { heartbeatId: "relay-registration-heartbeat-01", present: false, observationId: "after-relay" },
        { heartbeatId: "relay-runtime-heartbeat-01", present: false, observationId: "after-relay" },
      ],
    },
  })

  const cleanup = common("cleanup", times.cleanup, [
    observation("cleanup-provider", "hetzner-api", "verify rebuilt server allocation is retained", times.cleanup),
    observation("cleanup-host", "host-command", "verify old runtime cleanup", times.cleanup),
    observation("cleanup-cloud", "cloud-api", "verify stale control row cleanup", times.cleanup),
    observation("cleanup-relay", "relay-api", "verify stale heartbeat cleanup", times.cleanup),
    observation("cleanup-complete", "cleanup-command", "cleanup verification", times.cleanup),
  ], {
    allocation: { ...allocation, imageId: approvedImageId },
    cleanup: {
      recordId: "path1-cleanup-record-2209",
      completedAt: times.cleanup,
      oldRuntimeRetired: true,
      rebuiltServerPreserved: true,
      unrelatedResourcesUntouched: true,
      ownedTemporaryArtifactsRemoved: true,
      remainingOldIdentities: [],
    },
  })

  return { reviewed, before, rebuild, after, cleanup }
}

test("valid captured receipts prove a fresh-equivalent rebuild of the same allocation", () => {
  const report = verifyPath1RebuildEvidence(validEvidence())
  assert.equal(report.status, "pass")
  assert.equal(report.allocation.serverId, "server-9031")
  assert.equal(report.approvedImage.imageId, "hetzner-image-ubuntu-2604-approved")
  assert.equal(report.reviewedRelease.sourceCommit, "1".repeat(40))
  assert.deepEqual(report.retiredIdentityCounts, {
    serviceInstances: 1,
    processInstances: 1,
    stateRoots: 2,
    releaseInstances: 1,
    cloudRows: 2,
    relayHeartbeats: 2,
  })
})

test("a missing required receipt fails closed", () => {
  const evidence = validEvidence()
  evidence.rebuild = undefined
  assert.throws(
    () => verifyPath1RebuildEvidence(evidence),
    (error) => error instanceof RebuildEvidenceError && error.code === "receipt_missing",
  )
})

test("retained command and API capture files must match their recorded digests", async () => {
  const evidence = validEvidence()
  const directory = await mkdtemp(join(tmpdir(), "path1-rebuild-evidence-"))
  try {
    for (const [kind, receipt] of Object.entries(evidence)) {
      for (const item of receipt.observations) {
        const file = join(directory, `${kind}-${item.id}.json`)
        const bytes = Buffer.from(`captured:${item.id}`)
        await writeFile(file, bytes, { flag: "wx", mode: 0o600 })
        item.evidenceRef = `file:${file}`
        item.evidenceSha256 = `sha256:${createHash("sha256").update(bytes).digest("hex")}`
      }
    }
    await verifyEvidenceArtifactFiles(evidence)
    await writeFile(join(directory, "after-after-host.json"), "altered capture")
    await assert.rejects(
      () => verifyEvidenceArtifactFiles(evidence),
      (error) => error instanceof RebuildEvidenceError && error.code === "evidence_artifact_mismatch",
    )
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})

test("missing old-state absence evidence fails closed", () => {
  const evidence = validEvidence()
  evidence.after.absence.stateInstances.pop()
  assert.throws(
    () => verifyPath1RebuildEvidence(evidence),
    (error) => error instanceof RebuildEvidenceError && error.code === "residue_evidence_incomplete",
  )
})

test("stale post-rebuild receipts fail closed", () => {
  const evidence = validEvidence()
  evidence.after.capturedAt = times.complete
  for (const item of evidence.after.observations) item.capturedAt = times.complete
  assert.throws(
    () => verifyPath1RebuildEvidence(evidence),
    (error) => error instanceof RebuildEvidenceError && error.code === "receipt_stale",
  )
})

test("contradictory allocation, release, and residue identities fail closed", () => {
  const wrongAllocation = validEvidence()
  wrongAllocation.after.allocation.serverId = "different-server-9032"
  assert.throws(() => verifyPath1RebuildEvidence(wrongAllocation), (error) => error.code === "allocation_mismatch")

  const wrongRelease = validEvidence()
  wrongRelease.after.identity.release.digest = `sha256:${"c".repeat(64)}`
  assert.throws(() => verifyPath1RebuildEvidence(wrongRelease), (error) => error.code === "release_mismatch")

  const residue = validEvidence()
  residue.after.absence.heartbeats[0].present = true
  assert.throws(() => verifyPath1RebuildEvidence(residue), (error) => error.code === "old_identity_present")

  const otherCampaign = validEvidence()
  otherCampaign.cleanup.campaignId = "different-path1-rebuild-campaign"
  assert.throws(() => verifyPath1RebuildEvidence(otherCampaign), (error) => error.code === "campaign_mismatch")
})
