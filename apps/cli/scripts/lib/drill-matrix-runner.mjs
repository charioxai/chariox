import { spawn } from "node:child_process"

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

export async function runDrillMatrix({
  matrixName,
  scenarios,
  commandForScenario,
  cwd,
  continueOnFailure = false,
  dryRun = false,
}) {
  console.log(`[${matrixName}] selected ${scenarios.map((scenario) => scenario.id).join(", ")}`)
  if (dryRun) {
    for (const scenario of scenarios) {
      const { command, args } = commandForScenario(scenario)
      console.log(`[${matrixName}] dry-run ${scenario.id}: ${quoteDrillCommand(command, args)}`)
    }
    return []
  }

  const results = []
  for (const scenario of scenarios) {
    const result = await runMatrixScenario({ matrixName, scenario, commandForScenario, cwd })
    results.push(result)
    if (!result.ok && !continueOnFailure) break
  }

  console.log(`[${matrixName}] summary`)
  for (const result of results) {
    const expected = result.expectedFailure ? " expected_failure" : ""
    console.log(
      `  ${result.ok ? "pass" : "fail"} ${result.scenario.id}${expected} duration_ms=${result.durationMs}${
        result.reason ? ` ${result.reason}` : ""
      }`,
    )
  }

  return results
}

async function runMatrixScenario({ matrixName, scenario, commandForScenario, cwd }) {
  const start = Date.now()
  const { command, args } = commandForScenario(scenario)
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
      return { scenario, ok: false, durationMs, reason }
    }
    console.log(`[${matrixName}] pass ${scenario.id} duration_ms=${durationMs}`)
    return { scenario, ok: true, durationMs }
  }

  if (scenario.expectedFailure) {
    const expected = scenario.expectedOutputIncludes
    if (!expected || output.includes(expected)) {
      console.log(`[${matrixName}] pass ${scenario.id} expected_failure duration_ms=${durationMs}`)
      return { scenario, ok: true, durationMs, expectedFailure: true }
    }
    const reason = `expected failure output to include ${JSON.stringify(expected)}`
    console.error(`[${matrixName}] fail ${scenario.id} duration_ms=${durationMs} ${reason}`)
    return { scenario, ok: false, durationMs, reason }
  }

  const reason = status.error?.message ?? `code=${status.code} signal=${status.signal ?? "none"}`
  console.error(`[${matrixName}] fail ${scenario.id} duration_ms=${durationMs} ${reason}`)
  return { scenario, ok: false, durationMs, reason }
}

function requirementsFor(scenario) {
  return Array.isArray(scenario.requires) ? scenario.requires : []
}
