import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  validateCloudCompatibleDrillRuntimeAuthorityManifest,
  verifyDrillRuntimeAuthorityRegistryParity,
} from "./drill-runtime-authority-registry-parity.mjs"
import { drillRuntimeAuthorityManifest } from "./drill-runtime-authority-invariants.mjs"

test("accepts matching OSS and Cloud runtime authority registries", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-runtime-authority-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    await writeCloudRuntimeAuthorityRegistry(cloudRoot)

    await assert.doesNotReject(verifyDrillRuntimeAuthorityRegistryParity({ cloudRoot }))
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("accepts Cloud control-plane invariant alias and owner override", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-runtime-authority-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    await writeCloudRuntimeAuthorityRegistry(cloudRoot, {
      invariants: drillRuntimeAuthorityManifest().invariants.map((invariant) => invariant.id === "relay-cloud-transport-only"
        ? { ...invariant, id: "cloud-control-plane-only", owner: "cloud-deployment" }
        : invariant),
    })

    await assert.doesNotReject(verifyDrillRuntimeAuthorityRegistryParity({ cloudRoot }))
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("accepts Cloud control-plane aliases in embedded artifact manifests", () => {
  const manifest = drillRuntimeAuthorityManifest()
  const contextual = {
    ...manifest,
    invariants: manifest.invariants.map((invariant) => invariant.id === "relay-cloud-transport-only"
      ? { ...invariant, id: "cloud-control-plane-only", owner: "cloud-deployment" }
      : invariant),
  }

  assert.doesNotThrow(() => validateCloudCompatibleDrillRuntimeAuthorityManifest(contextual))
})

test("rejects missing Cloud runtime authority registry exports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-runtime-authority-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-runtime-authority-invariants.mjs")
    await mkdir(path.dirname(registryPath), { recursive: true })
    await writeFile(registryPath, "export const noManifest = true\n", "utf8")

    await assert.rejects(
      verifyDrillRuntimeAuthorityRegistryParity({ cloudRoot }),
      /requires cloudRuntimeAuthorityManifest/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects missing Cloud runtime authority invariants", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-runtime-authority-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    await writeCloudRuntimeAuthorityRegistry(cloudRoot, {
      invariants: drillRuntimeAuthorityManifest().invariants.filter((invariant) => invariant.id !== "worker-execution-authority"),
    })

    await assert.rejects(
      verifyDrillRuntimeAuthorityRegistryParity({ cloudRoot }),
      /runtime authority registry parity failed: worker-execution-authority: missing Cloud runtime authority invariant/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects Cloud runtime authority owner drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-runtime-authority-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    await writeCloudRuntimeAuthorityRegistry(cloudRoot, {
      invariants: drillRuntimeAuthorityManifest().invariants.map((invariant) => invariant.id === "worker-execution-authority"
        ? { ...invariant, owner: "kernel-authority" }
        : invariant),
    })

    await assert.rejects(
      verifyDrillRuntimeAuthorityRegistryParity({ cloudRoot }),
      /worker-execution-authority: owner OSS=worker-kernel Cloud=kernel-authority/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects Cloud runtime authority signal drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-runtime-authority-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    await writeCloudRuntimeAuthorityRegistry(cloudRoot, {
      invariants: drillRuntimeAuthorityManifest().invariants.map((invariant) => invariant.id === "home-session-authority"
        ? { ...invariant, requiredRuntimeSignals: ["session-authority"] }
        : invariant),
    })

    await assert.rejects(
      verifyDrillRuntimeAuthorityRegistryParity({ cloudRoot }),
      /home-session-authority: requiredRuntimeSignals/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects unsupported Cloud runtime authority schemas", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-runtime-authority-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    await writeCloudRuntimeAuthorityRegistry(cloudRoot, {
      schema: "chariox.cloud.runtime_authority_invariants.v1",
    })

    await assert.rejects(
      verifyDrillRuntimeAuthorityRegistryParity({ cloudRoot }),
      /Cloud runtime authority registry has unsupported schema "chariox.cloud.runtime_authority_invariants.v1"/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects duplicate Cloud runtime authority invariant ids after aliases", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-runtime-authority-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    const relayInvariant = drillRuntimeAuthorityManifest().invariants.find((invariant) => invariant.id === "relay-cloud-transport-only")
    await writeCloudRuntimeAuthorityRegistry(cloudRoot, {
      invariants: [
        ...drillRuntimeAuthorityManifest().invariants,
        { ...relayInvariant, id: "cloud-control-plane-only", owner: "cloud-deployment" },
      ],
    })

    await assert.rejects(
      verifyDrillRuntimeAuthorityRegistryParity({ cloudRoot }),
      /Cloud runtime authority registry\.invariants\[\d+\] duplicates invariant "relay-cloud-transport-only"/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function writeCloudRuntimeAuthorityRegistry(cloudRoot, {
  schema = "chariox.drill.runtime_authority_invariants.v1",
  invariants = drillRuntimeAuthorityManifest().invariants,
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-runtime-authority-invariants.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudRuntimeAuthorityManifest() {",
    `  return { schema: ${JSON.stringify(schema)}, invariants: ${JSON.stringify(invariants)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}
