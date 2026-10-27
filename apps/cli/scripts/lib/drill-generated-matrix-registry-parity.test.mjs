import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { verifyDrillGeneratedMatrixRegistryParity } from "./drill-generated-matrix-registry-parity.mjs"
import { drillGeneratedMatrixNamesManifest } from "./drill-generated-matrix-names.mjs"

test("accepts matching OSS and Cloud generated matrix registries", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-generated-matrix-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    await writeCloudGeneratedMatrixRegistry(cloudRoot)

    await assert.doesNotReject(verifyDrillGeneratedMatrixRegistryParity({ cloudRoot }))
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects drifted Cloud generated matrix registries", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-generated-matrix-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    await writeCloudGeneratedMatrixRegistry(cloudRoot, {
      matrices: [
        { name: "cloud-slice-runtime-matrix", repo: "cloud" },
        { name: "slice-runtime-matrix", repo: "oss" },
      ],
    })

    await assert.rejects(
      verifyDrillGeneratedMatrixRegistryParity({ cloudRoot }),
      /generated matrix registry parity failed: .*workspace-live-sync-matrix/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects missing Cloud generated matrix registry exports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-generated-matrix-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-drill-generated-matrix-names.mjs")
    await mkdir(path.dirname(registryPath), { recursive: true })
    await writeFile(registryPath, "export const noManifest = true\n", "utf8")

    await assert.rejects(
      verifyDrillGeneratedMatrixRegistryParity({ cloudRoot }),
      /requires cloudDrillGeneratedMatrixNamesManifest/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects malformed Cloud generated matrix manifests", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-generated-matrix-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    await writeCloudGeneratedMatrixRegistry(cloudRoot, {
      matrices: [{ name: "workspace-live-sync-matrix" }],
    })

    await assert.rejects(
      verifyDrillGeneratedMatrixRegistryParity({ cloudRoot }),
      /Cloud generated matrix registry\.matrices\[0\] has invalid repo/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects unsupported Cloud generated matrix registry schemas", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-generated-matrix-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    await writeCloudGeneratedMatrixRegistry(cloudRoot, {
      schema: "chariox.drill.generated_matrix_names.v1",
    })

    await assert.rejects(
      verifyDrillGeneratedMatrixRegistryParity({ cloudRoot }),
      /Cloud generated matrix registry has unsupported schema "chariox.drill.generated_matrix_names.v1"/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects duplicate Cloud generated matrix names", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-generated-matrix-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    await writeCloudGeneratedMatrixRegistry(cloudRoot, {
      matrices: [
        ...drillGeneratedMatrixNamesManifest().matrices,
        { name: "workspace-live-sync-matrix", repo: "oss" },
      ],
    })

    await assert.rejects(
      verifyDrillGeneratedMatrixRegistryParity({ cloudRoot }),
      /Cloud generated matrix registry\.matrices\[8\] duplicates matrix "workspace-live-sync-matrix"/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function writeCloudGeneratedMatrixRegistry(cloudRoot, {
  schema = "chariox.cloud.drill.generated_matrix_names.v1",
  matrices = drillGeneratedMatrixNamesManifest().matrices,
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-drill-generated-matrix-names.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudDrillGeneratedMatrixNamesManifest() {",
    `  return { schema: ${JSON.stringify(schema)}, matrices: ${JSON.stringify(matrices)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}
