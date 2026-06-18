import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { verifyDrillFailureTaxonomyRegistryParity } from "./drill-failure-taxonomy-registry-parity.mjs"
import { drillFailureTaxonomyManifest } from "./drill-failure-taxonomy.mjs"

test("accepts Cloud failure taxonomy classifications known by OSS", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-failure-taxonomy-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillFailureTaxonomyManifest()
    await writeCloudFailureTaxonomyRegistry(cloudRoot, {
      classifications: manifest.classifications
        .filter((classification) => [
          "docker-runtime",
          "kernel-authority",
          "runtime-projection-health",
          "workspace-live-sync-conflict",
        ].includes(classification.kind))
        .map((classification) => classification.kind === "docker-runtime"
          ? { ...classification, owner: "worker-kernel" }
          : classification),
    })

    await assert.doesNotReject(verifyDrillFailureTaxonomyRegistryParity({ cloudRoot }))
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects Cloud failure classifications unknown to OSS", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-failure-taxonomy-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillFailureTaxonomyManifest()
    await writeCloudFailureTaxonomyRegistry(cloudRoot, {
      classifications: [
        manifest.classifications.find((classification) => classification.kind === "kernel-authority"),
        {
          kind: "future-cloud-only-classification",
          owner: "kernel-authority",
          nextAction: "inspect future diagnostics",
        },
      ],
    })

    await assert.rejects(
      verifyDrillFailureTaxonomyRegistryParity({ cloudRoot }),
      /failure taxonomy registry parity failed: future-cloud-only-classification: unknown in OSS failure taxonomy/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects Cloud failure classification owner drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-failure-taxonomy-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillFailureTaxonomyManifest()
    await writeCloudFailureTaxonomyRegistry(cloudRoot, {
      classifications: manifest.classifications
        .filter((classification) => classification.kind === "kernel-authority")
        .map((classification) => ({ ...classification, owner: "runtime-state" })),
    })

    await assert.rejects(
      verifyDrillFailureTaxonomyRegistryParity({ cloudRoot }),
      /failure taxonomy registry parity failed: kernel-authority: owner OSS=kernel-authority Cloud=runtime-state/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects missing Cloud failure taxonomy registry exports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-failure-taxonomy-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-failure-taxonomy.mjs")
    await mkdir(path.dirname(registryPath), { recursive: true })
    await writeFile(registryPath, "export const noManifest = true\n", "utf8")

    await assert.rejects(
      verifyDrillFailureTaxonomyRegistryParity({ cloudRoot }),
      /requires cloudFailureTaxonomyManifest/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects malformed Cloud failure taxonomy manifests", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-failure-taxonomy-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeCloudFailureTaxonomyRegistry(cloudRoot, {
      classifications: [{ kind: "kernel-authority", owner: "kernel-authority" }],
    })

    await assert.rejects(
      verifyDrillFailureTaxonomyRegistryParity({ cloudRoot }),
      /Cloud failure taxonomy registry\.classifications\[0\] has invalid nextAction/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function writeCloudFailureTaxonomyRegistry(cloudRoot, {
  classifications,
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-failure-taxonomy.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudFailureTaxonomyManifest() {",
    `  return { schema: "arroba.drill.failure_taxonomy.v1", target: "scenario", classifications: ${JSON.stringify(classifications)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}
