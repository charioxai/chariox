import { spawn } from "node:child_process"
import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"
import { writeDrillArtifactIndex } from "./drill-artifacts.mjs"
import { classifyDrillChildFailure } from "./drill-child-process.mjs"
import { validateDrillMatrixReport } from "./drill-matrix-report.mjs"
import { looksLikeDrillSecretValue } from "./drill-secrets.mjs"

export function parseDrillScenarioIds(value) {
  if (value == null) return null
  const values = Array.isArray(value) ? value : String(value).split(",")
  return values.map((id) => id.trim()).filter(Boolean)
}

export function selectDrillMatrixScenarios({
  scenarios,
  requestedIds = null,
  enabledRequirements = new Set(),
  requirementLabels = {},
}) {
  validateDrillMatrixScenarioDefinitions(scenarios, { requireDescription: false })
  const known = new Map(scenarios.map((scenario) => [scenario.id, scenario]))
  const selected = requestedIds
    ? requestedIds.map((id) => {
        const scenario = known.get(id)
        if (!scenario) throw new Error(`unknown scenario id: ${id}`)
        return scenario
      })
    : scenarios.filter((scenario) => requirementsFor(scenario).every((requirement) => enabledRequirements.has(requirement)))

  for (const scenario of selected) {
    const missing = requirementsFor(scenario).filter((requirement) => !enabledRequirements.has(requirement))
    if (missing.length > 0) {
      const label = requirementLabels[missing[0]] ?? missing[0]
      throw new Error(`${scenario.id} requires ${label}`)
    }
  }

  if (selected.length === 0) throw new Error("no scenarios selected")
  return selected
}

export function quoteDrillCommand(command, args) {
  return [command, ...args].map((part) => (/[ "'\\]/.test(part) ? JSON.stringify(part) : part)).join(" ")
}

export function defaultDrillMatrixReportPath(matrixName, { rootDir = process.cwd(), now = new Date() } = {}) {
  const safeName = String(matrixName)
    .trim()
    .replace(/[^a-zA-Z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    || "matrix"
  const stamp = now.toISOString().replace(/[:.]/g, "-")
  return path.join(rootDir, ".artifacts", "drill-matrices", safeName, `${stamp}.json`)
}

export function defaultDrillMatrixArtifactIndexPath(reportPath) {
  if (!nonEmptyString(reportPath)) throw new Error("matrix reportPath is required")
  const extension = path.extname(reportPath)
  const baseName = path.basename(reportPath, extension)
  return path.join(path.dirname(reportPath), `${baseName}-artifacts`, "arroba-drill-artifacts.json")
}

export function extractDrillArtifactHints(text) {
  const hints = new Set()
  for (const line of String(text ?? "").split("\n")) {
    collectArtifactHintsFromJsonLine(hints, line)
    collectArtifactHintsFromTextLine(hints, line)
  }
  return [...hints].sort()
}

export async function runDrillMatrix({
  matrixName,
  scenarios,
  commandForScenario,
  cwd,
  continueOnFailure = false,
  dryRun = false,
  reportPath = null,
  artifactIndexPath = null,
  metadata = {},
}) {
  validateDrillMatrixScenarioDefinitions(scenarios)
  validateCommandFactory(commandForScenario)
  if (artifactIndexPath && !reportPath) {
    throw new Error("matrix artifactIndexPath requires reportPath")
  }
  const preparedScenarios = prepareMatrixScenarioCommands(scenarios, commandForScenario)
  const startedAt = new Date()
  console.log(`[${matrixName}] selected ${scenarios.map((scenario) => scenario.id).join(", ")}`)
  if (dryRun) {
    const results = []
    for (const { scenario, command, args } of preparedScenarios) {
      console.log(`[${matrixName}] dry-run ${scenario.id}: ${quoteDrillCommand(command, args)}`)
      results.push({
        scenario,
        ok: true,
        dryRun: true,
        command,
        args,
        durationMs: 0,
      })
    }
    await maybeWriteMatrixReport({ reportPath, artifactIndexPath, matrixName, startedAt, results, dryRun, metadata })
    return results
  }

  const results = []
  for (let index = 0; index < preparedScenarios.length; index += 1) {
    const prepared = preparedScenarios[index]
    const result = await runMatrixScenario({ matrixName, ...prepared, cwd })
    results.push(result)
    if (!result.ok && !continueOnFailure) {
      for (const skipped of preparedScenarios.slice(index + 1)) {
        results.push({
          scenario: skipped.scenario,
          ok: true,
          skipped: true,
          command: skipped.command,
          args: skipped.args,
          durationMs: 0,
          reason: "skipped after previous failure",
        })
      }
      break
    }
  }

  console.log(`[${matrixName}] summary`)
  for (const result of results) {
    const expected = result.expectedFailure ? " expected_failure" : ""
    const classification = result.classification ? ` classification=${result.classification}` : ""
    const status = result.skipped ? "skip" : result.ok ? "pass" : "fail"
    console.log(
      `  ${status} ${result.scenario.id}${expected} duration_ms=${result.durationMs}${classification}${
        result.reason ? ` ${result.reason}` : ""
      }`,
    )
  }

  await maybeWriteMatrixReport({ reportPath, artifactIndexPath, matrixName, startedAt, results, dryRun, metadata })
  return results
}

async function runMatrixScenario({ matrixName, scenario, command, args, cwd }) {
  const start = Date.now()
  console.log(`[${matrixName}] start ${scenario.id}: ${scenario.description}`)
  console.log(`[${matrixName}] command ${quoteDrillCommand(command, args)}`)

  let output = ""
  const appendOutput = (chunk, stream) => {
    const text = chunk.toString()
    stream.write(text)
    output += text
    if (output.length > 2_000_000) output = output.slice(-1_000_000)
  }

  const status = await new Promise((resolve) => {
    const child = spawn(command, args, { cwd, stdio: ["ignore", "pipe", "pipe"] })
    child.stdout.on("data", (chunk) => appendOutput(chunk, process.stdout))
    child.stderr.on("data", (chunk) => appendOutput(chunk, process.stderr))
    child.on("exit", (code, signal) => resolve({ code, signal }))
    child.on("error", (error) => resolve({ code: 1, signal: null, error }))
  })

  const durationMs = Date.now() - start
  if (status.code === 0) {
    if (scenario.expectedFailure) {
      const reason = "expected unsupported failure but scenario exited successfully"
      console.error(`[${matrixName}] fail ${scenario.id} duration_ms=${durationMs} ${reason}`)
      return { scenario, ok: false, durationMs, reason, command, args }
    }
    console.log(`[${matrixName}] pass ${scenario.id} duration_ms=${durationMs}`)
    return { scenario, ok: true, durationMs, command, args }
  }

  if (scenario.expectedFailure) {
    const expected = scenario.expectedOutputIncludes
    if (!expected || output.includes(expected)) {
      console.log(`[${matrixName}] pass ${scenario.id} expected_failure duration_ms=${durationMs}`)
      return { scenario, ok: true, durationMs, expectedFailure: true, classification: "expected-failure", command, args, artifactHints: extractDrillArtifactHints(output) }
    }
    const reason = `expected failure output to include ${JSON.stringify(expected)}`
    const classification = classifyDrillChildFailure(output)
    console.error(`[${matrixName}] fail ${scenario.id} duration_ms=${durationMs} classification=${classification} ${reason}`)
    return { scenario, ok: false, durationMs, reason, classification, command, args, artifactHints: extractDrillArtifactHints(output) }
  }

  const reason = status.error?.message ?? `code=${status.code} signal=${status.signal ?? "none"}`
  const failureOutput = `${output}\n${status.error?.message ?? ""}`
  const classification = classifyDrillChildFailure(failureOutput)
  console.error(`[${matrixName}] fail ${scenario.id} duration_ms=${durationMs} classification=${classification} ${reason}`)
  return { scenario, ok: false, durationMs, reason, classification, command, args, artifactHints: extractDrillArtifactHints(failureOutput) }
}

function prepareMatrixScenarioCommands(scenarios, commandForScenario) {
  return scenarios.map((scenario) => {
    const commandSpec = commandForScenario(scenario)
    validateMatrixScenarioCommand(commandSpec, `${scenario.id} command`)
    return { scenario, command: commandSpec.command, args: commandSpec.args }
  })
}

export function validateDrillMatrixScenarioDefinitions(scenarios, { requireDescription = true } = {}) {
  if (!Array.isArray(scenarios)) {
    throw new Error("matrix scenarios must be an array")
  }
  if (scenarios.length === 0) {
    throw new Error("matrix scenarios must not be empty")
  }
  const seen = new Set()
  for (const [index, scenario] of scenarios.entries()) {
    const source = `matrix scenarios[${index}]`
    if (!scenario || typeof scenario !== "object" || Array.isArray(scenario)) {
      throw new Error(`${source} is not an object`)
    }
    if (!nonEmptyString(scenario.id)) {
      throw new Error(`${source} is missing id`)
    }
    if (seen.has(scenario.id)) {
      throw new Error(`duplicate matrix scenario id: ${scenario.id}`)
    }
    seen.add(scenario.id)
    if (requireDescription && !nonEmptyString(scenario.description)) {
      throw new Error(`${source} is missing description`)
    }
    if (scenario.requires !== undefined && (
      !Array.isArray(scenario.requires)
      || !scenario.requires.every((requirement) => nonEmptyString(requirement))
    )) {
      throw new Error(`${source} has invalid requires`)
    }
    if (scenario.exitCriteria !== undefined && !validExitCriteriaDefinition(scenario.exitCriteria)) {
      throw new Error(`${source} has invalid exitCriteria`)
    }
    if (scenario.expectedFailure !== undefined && typeof scenario.expectedFailure !== "boolean") {
      throw new Error(`${source} has invalid expectedFailure`)
    }
    if (scenario.expectedOutputIncludes !== undefined && !nonEmptyString(scenario.expectedOutputIncludes)) {
      throw new Error(`${source} has invalid expectedOutputIncludes`)
    }
  }
}

function validateCommandFactory(commandForScenario) {
  if (typeof commandForScenario !== "function") {
    throw new Error("matrix commandForScenario must be a function")
  }
}

function validateMatrixScenarioCommand(commandSpec, source) {
  if (!commandSpec || typeof commandSpec !== "object" || Array.isArray(commandSpec)) {
    throw new Error(`${source} is not an object`)
  }
  if (!nonEmptyString(commandSpec.command)) {
    throw new Error(`${source} is missing command`)
  }
  if (!Array.isArray(commandSpec.args) || !commandSpec.args.every((arg) => typeof arg === "string")) {
    throw new Error(`${source} has invalid args`)
  }
}

function requirementsFor(scenario) {
  return Array.isArray(scenario.requires) ? scenario.requires : []
}

function validExitCriteriaDefinition(value) {
  if (typeof value === "string") return value.trim().length > 0
  return Array.isArray(value) && value.every((criterion) => nonEmptyString(criterion))
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}

function exitCriteriaFor(scenario) {
  if (Array.isArray(scenario.exitCriteria)) {
    return scenario.exitCriteria.filter((criterion) => typeof criterion === "string" && criterion.trim().length > 0)
  }
  if (typeof scenario.exitCriteria === "string" && scenario.exitCriteria.trim()) {
    return [scenario.exitCriteria.trim()]
  }
  return []
}

async function maybeWriteMatrixReport({ reportPath, artifactIndexPath, matrixName, startedAt, results, dryRun, metadata }) {
  if (!reportPath) return
  const completedAt = new Date()
  const report = {
    schema: "arroba.drill.matrix.v1",
    matrix: matrixName,
    status: dryRun ? "dry-run" : results.every((result) => result.ok) ? "passed" : "failed",
    dryRun,
    startedAt: startedAt.toISOString(),
    completedAt: completedAt.toISOString(),
    durationMs: completedAt.getTime() - startedAt.getTime(),
    metadata,
    scenarios: results.map((result) => ({
      id: result.scenario.id,
      description: result.scenario.description,
      requires: requirementsFor(result.scenario),
      exitCriteria: exitCriteriaFor(result.scenario),
      status: result.dryRun ? "dry-run" : result.skipped ? "skipped" : result.ok ? "passed" : "failed",
      expectedFailure: Boolean(result.expectedFailure),
      classification: result.classification ?? null,
      durationMs: result.durationMs,
      reason: result.reason ?? null,
      command: result.command,
      args: result.args,
      artifactHints: Array.isArray(result.artifactHints) ? result.artifactHints : [],
    })),
  }
  validateDrillMatrixReport(report, reportPath)
  await mkdir(path.dirname(reportPath), { recursive: true })
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8")
  if (artifactIndexPath) {
    await writeDrillArtifactIndex({
      rootDir: path.dirname(reportPath),
      artifacts: [path.basename(reportPath)],
      indexPath: artifactIndexPath,
      metadata: { matrix: matrixName },
    })
  }
  console.log(`[${matrixName}] report ${reportPath}`)
}

function collectArtifactHintsFromJsonLine(hints, line) {
  const jsonStart = line.indexOf("{")
  if (jsonStart === -1) return
  try {
    collectArtifactHintsFromValue(hints, JSON.parse(line.slice(jsonStart)))
  } catch {
    // Not every drill log line with braces is JSON.
  }
}

function collectArtifactHintsFromValue(hints, value, key = "") {
  if (typeof value === "string") {
    if (isArtifactKey(key) && looksLikeArtifactPath(value)) hints.add(value)
    return
  }
  if (!value || typeof value !== "object") return
  if (Array.isArray(value)) {
    for (const item of value) collectArtifactHintsFromValue(hints, item, key)
    return
  }
  for (const [childKey, childValue] of Object.entries(value)) {
    collectArtifactHintsFromValue(hints, childValue, childKey)
  }
}

function collectArtifactHintsFromTextLine(hints, line) {
  const patterns = [
    /\bartifacts?(?:\s+\w+){0,4}\s+(?:at|root|kept|preserved|retained):?\s+(\/[^\s"']+)/ig,
    /\b(?:artifactRoot|rootDir|manifestPath)\s*[=:]\s*(\/[^\s"',}]+)/g,
  ]
  for (const pattern of patterns) {
    for (const match of line.matchAll(pattern)) {
      if (looksLikeArtifactPath(match[1])) hints.add(match[1])
    }
  }
}

function isArtifactKey(key) {
  return /artifact|rootDir|manifestPath/i.test(key)
}

function looksLikeArtifactPath(value) {
  return typeof value === "string"
    && value.length < 500
    && !looksLikeDrillSecretValue(value)
    && (/^\/[^ ]+/.test(value) || value.includes(".artifacts") || value.includes("arroba-drill"))
}
