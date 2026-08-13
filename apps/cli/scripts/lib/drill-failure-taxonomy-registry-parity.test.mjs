import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  validateCloudCompatibleDrillFailureTaxonomyManifest,
  verifyDrillFailureTaxonomyRegistryParity,
} from "./drill-failure-taxonomy-registry-parity.mjs"
import { drillFailureTaxonomyManifest } from "./drill-failure-taxonomy.mjs"

test("accepts Cloud failure taxonomy classifications known by OSS", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-failure-taxonomy-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    const manifest = drillFailureTaxonomyManifest()
    await writeCloudFailureTaxonomyRegistry(cloudRoot, {
      classifications: manifest.classifications
        .map((classification) => classification.kind === "docker-runtime"
          ? { ...classification, owner: "worker-kernel" }
          : classification),
    })

    await assert.doesNotReject(verifyDrillFailureTaxonomyRegistryParity({ cloudRoot }))
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("accepts Cloud contextual diagnostics in embedded artifact manifests", () => {
  const manifest = drillFailureTaxonomyManifest()
  const contextual = {
    ...manifest,
    classifications: manifest.classifications.map((classification) => {
      if (classification.kind === "cloud-runtime") {
        return { ...classification, nextAction: "inspect hosted Cloud deployment logs, then rerun the scenario" }
      }
      if (classification.kind === "docker-runtime") {
        return {
          ...classification,
          owner: "worker-kernel",
          nextAction: "inspect the slice container logs, then rerun the scenario",
        }
      }
      return classification
    }),
  }

  assert.doesNotThrow(() => validateCloudCompatibleDrillFailureTaxonomyManifest(contextual))
})

test("rejects missing Cloud failure classifications required by OSS diagnostics", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-failure-taxonomy-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    const manifest = drillFailureTaxonomyManifest()
    await writeCloudFailureTaxonomyRegistry(cloudRoot, {
      classifications: manifest.classifications.filter((classification) => classification.kind !== "remote-extension-sync"),
    })

    await assert.rejects(
      verifyDrillFailureTaxonomyRegistryParity({ cloudRoot }),
      /failure taxonomy registry parity failed: remote-extension-sync: missing Cloud failure taxonomy manifest entry/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects Cloud failure classifications unknown to OSS", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-failure-taxonomy-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    const manifest = drillFailureTaxonomyManifest()
    await writeCloudFailureTaxonomyRegistry(cloudRoot, {
      classifications: [
        ...manifest.classifications,
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
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-failure-taxonomy-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    const manifest = drillFailureTaxonomyManifest()
    await writeCloudFailureTaxonomyRegistry(cloudRoot, {
      classifications: manifest.classifications
        .map((classification) => classification.kind === "kernel-authority"
          ? { ...classification, owner: "runtime-state" }
          : classification),
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
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-failure-taxonomy-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
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
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-failure-taxonomy-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
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

test("rejects duplicate Cloud failure classifications", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-failure-taxonomy-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    await writeCloudFailureTaxonomyRegistry(cloudRoot, {
      classifications: [
        ...drillFailureTaxonomyManifest().classifications,
        drillFailureTaxonomyManifest().classifications.find((classification) => classification.kind === "kernel-authority"),
      ],
    })

    await assert.rejects(
      verifyDrillFailureTaxonomyRegistryParity({ cloudRoot }),
      /Cloud failure taxonomy registry\.classifications\[\d+\] duplicates classification "kernel-authority"/,
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
    `  return { schema: "chariox.drill.failure_taxonomy.v1", target: "scenario", classifications: ${JSON.stringify(classifications)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}
