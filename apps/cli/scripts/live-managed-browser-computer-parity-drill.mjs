#!/usr/bin/env node

import { mkdir, rm } from "node:fs/promises"
import path from "node:path"

import { writeDrillJsonArtifactOutput } from "./lib/drill-artifacts.mjs"
import {
  loadReviewedManagedParityAdapterModule,
  parseManagedBrowserComputerParityArgs,
  readPrivateManagedParityConfig,
} from "./lib/managed-browser-computer-parity-cli.mjs"
import { runManagedBrowserComputerParityHarness } from "./lib/managed-browser-computer-parity-harness.mjs"

const repoRoot = path.resolve(import.meta.dirname, "../../..")

let incompleteRunDir = null
const interruption = new AbortController()
const interrupt = () => interruption.abort()
process.once("SIGINT", interrupt)
process.once("SIGTERM", interrupt)
try {
  const options = parseManagedBrowserComputerParityArgs(process.argv.slice(2), { repoRoot })
  const config = await readPrivateManagedParityConfig(options.configPath)
  const { imported, verification: adapterVerification } = await loadReviewedManagedParityAdapterModule(
    options.transportModulePath,
    config.adapter,
  )
  if (typeof imported.createManagedBrowserComputerParityTransport !== "function") {
    throw new Error("transport module must export createManagedBrowserComputerParityTransport")
  }
  const runDir = path.join(options.evidenceRoot, config.runId)
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(config.runId ?? "")) {
    throw new Error("managed parity run id is invalid")
  }
  incompleteRunDir = runDir
  await mkdir(runDir, { recursive: true, mode: 0o700 })
  const product = await imported.createManagedBrowserComputerParityTransport({
    evidenceRoot: runDir,
  })
  if (!product || product.transport === product.inspector) {
    throw new Error("reviewed adapter must return distinct transport and independent inspector objects")
  }
  const report = await runManagedBrowserComputerParityHarness({
    config,
    transport: product.transport,
    inspector: product.inspector,
    adapterVerification,
    signal: interruption.signal,
  })
  const resultPath = path.join(runDir, "managed-browser-computer-parity.json")
  const artifactIndexPath = path.join(runDir, "chariox-drill-artifacts.json")
  await writeDrillJsonArtifactOutput({
    outputPath: resultPath,
    value: report,
    artifactIndexPath,
  })
  incompleteRunDir = null
  process.stdout.write(`${JSON.stringify({
    schema: report.schema,
    status: report.status,
    runId: report.runId,
    resultPath,
    artifactIndexPath,
    failureCode: report.failure?.code ?? null,
  })}\n`)
  if (report.status !== "passed") process.exitCode = 1
} catch {
  if (incompleteRunDir) await rm(incompleteRunDir, { recursive: true, force: true }).catch(() => {})
  process.stderr.write("managed browser/computer parity harness failed before producing validated evidence\n")
  process.exitCode = 1
} finally {
  process.removeListener("SIGINT", interrupt)
  process.removeListener("SIGTERM", interrupt)
}
