import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { drillChaosContractManifest } from "./drill-chaos-contract.mjs"
import { verifyDrillChaosContractRegistryParity } from "./drill-chaos-contract-registry-parity.mjs"

test("accepts matching OSS and Cloud chaos contracts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-chaos-contract-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    await writeCloudChaosContractRegistry(cloudRoot)
    await assert.doesNotReject(verifyDrillChaosContractRegistryParity({ cloudRoot }))
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects drifted Cloud chaos contracts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-chaos-contract-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    const manifest = drillChaosContractManifest()
    await writeCloudChaosContractRegistry(cloudRoot, {
      ...manifest,
      faultKinds: manifest.faultKinds.filter((kind) => kind !== "process-death"),
    })
    await assert.rejects(
      verifyDrillChaosContractRegistryParity({ cloudRoot }),
      /chaos contract registry parity failed: .*process-death/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects missing Cloud chaos contract manifest exports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-chaos-contract-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-chaos-contract.mjs")
    await mkdir(path.dirname(registryPath), { recursive: true })
    await writeFile(registryPath, "export const noManifest = true\n", "utf8")
    await assert.rejects(
      verifyDrillChaosContractRegistryParity({ cloudRoot }),
      /requires cloudDrillChaosContractManifest/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects malformed Cloud chaos contract manifests", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-chaos-contract-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    await writeCloudChaosContractRegistry(cloudRoot, {
      ...drillChaosContractManifest(),
      invariantIds: ["bounded-queues", "bounded-queues"],
    })
    await assert.rejects(
      verifyDrillChaosContractRegistryParity({ cloudRoot }),
      /invariantIds contains duplicate values/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function writeCloudChaosContractRegistry(cloudRoot, manifest = drillChaosContractManifest()) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-chaos-contract.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudDrillChaosContractManifest() {",
    `  return ${JSON.stringify(manifest)}`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}
