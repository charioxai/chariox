import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { verifyDrillRuntimeSignalRegistryParity } from "./drill-runtime-signal-registry-parity.mjs"
import { drillRuntimeSignalsManifest } from "./drill-runtime-signals.mjs"

test("accepts matching OSS and Cloud runtime signal registries", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-runtime-signal-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeCloudRuntimeSignalsRegistry(cloudRoot)

    await assert.doesNotReject(verifyDrillRuntimeSignalRegistryParity({ cloudRoot }))
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects drifted Cloud runtime signal registries", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-runtime-signal-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillRuntimeSignalsManifest()
    await writeCloudRuntimeSignalsRegistry(cloudRoot, {
      signals: manifest.signals.map((signal) => signal.id === "lease-health"
        ? { ...signal, description: "Wrong lease health description." }
        : signal),
    })

    await assert.rejects(
      verifyDrillRuntimeSignalRegistryParity({ cloudRoot }),
      /runtime signal registry parity failed: .*lease-health/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects missing Cloud runtime signal registry exports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-runtime-signal-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-runtime-signals.mjs")
    await mkdir(path.dirname(registryPath), { recursive: true })
    await writeFile(registryPath, "export const noManifest = true\n", "utf8")

    await assert.rejects(
      verifyDrillRuntimeSignalRegistryParity({ cloudRoot }),
      /requires cloudRuntimeSignalsManifest/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects malformed Cloud runtime signal manifests", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-runtime-signal-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeCloudRuntimeSignalsRegistry(cloudRoot, {
      signals: [{ id: "session-authority", owner: "kernel-authority" }],
    })

    await assert.rejects(
      verifyDrillRuntimeSignalRegistryParity({ cloudRoot }),
      /Cloud runtime signal registry\.signals\[0\] has invalid description/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects duplicate Cloud runtime signal ids", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-runtime-signal-parity-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeCloudRuntimeSignalsRegistry(cloudRoot, {
      signals: [
        ...drillRuntimeSignalsManifest().signals,
        drillRuntimeSignalsManifest().signals.find((signal) => signal.id === "lease-health"),
      ],
    })

    await assert.rejects(
      verifyDrillRuntimeSignalRegistryParity({ cloudRoot }),
      /Cloud runtime signal registry\.signals\[\d+\] duplicates signal "lease-health"/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function writeCloudRuntimeSignalsRegistry(cloudRoot, {
  signals = drillRuntimeSignalsManifest().signals,
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-runtime-signals.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudRuntimeSignalsManifest() {",
    `  return { schema: "arroba.drill.runtime_signals.v1", signals: ${JSON.stringify(signals)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}
