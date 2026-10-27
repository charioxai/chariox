#!/usr/bin/env node

import assert from "node:assert/strict"
import { mkdir, open, realpath } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"
import {
  drillEScenarioCleanupBudgetMs,
  drillEScenarioDetachReserveMs,
  runDrillEScenario as runDrillEScenarioStateful,
} from "./lib/drill-e-scenario-runner.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const defaultTimeoutMs = 180_000
const maxTimeoutMs = 300_000
const minPollMs = 250
const maxPollMs = 1_000
const maxEvidenceBytes = 4 * 1024 * 1024
const drillECheckNames = [
  "twoAgentTabReads",
  "sameTabSerialization",
  "independentTabConcurrency",
  "humanTakeover",
]
const diagnosticStages = new Set([
  "attach",
  "session_preflight",
  "environment_preflight",
  "baseline_action_history",
  "read_prompt_submission",
  "read_overlap_observation",
  "first_mutation_prompt_submission",
  "first_mutation_observation",
  "second_mutation_prompt_submission",
  "same_tab_queue_observation",
  "human_takeover_request",
  "human_takeover_observation",
  "final_environment_observation",
  "action_history",
  "evidence_verification",
  "runtime_setup",
  "evidence_write",
  "scenario",
])
const cleanupStages = new Set([
  "starting", "attachment_identity_unavailable", "owned_prompts", "takeover", "detach", "finished",
  "cleanup_runner",
])
const cleanupFailureCodes = new Set([
  "owned_prompt_state_unavailable",
  "owned_prompt_state_projection_missing",
  "submitted_prompt_id_unavailable",
  "owned_prompt_identity_mismatch",
  "owned_active_prompt_cancel_by_id_unavailable",
  "owned_queued_prompt_not_confirmed_settled",
  "takeover_state_unavailable",
  "takeover_environment_changed",
  "pending_takeover_cannot_be_retracted",
  "takeover_request_result_unknown",
  "takeover_actor_identity_unavailable",
  "owned_takeover_release_failed",
  "session_attachment_identity_unavailable",
  "owned_prompt_cleanup_failed",
  "owned_takeover_cleanup_failed",
  "session_detach_failed",
  "cleanup_runner_failed",
])
const cleanupPromptPurposes = new Set([
  "same-tab reads A",
  "same-tab reads B",
  "independent-tab work C",
  "first mutation A",
  "independent mutation C",
  "second mutation B",
])
const cleanupPromptStatuses = new Set([
  "unresolved", "identity_unknown", "settled", "identity_mismatch", "active_cancel_seam_missing",
  "queued", "queued_cancel_unconfirmed", "queued_cancelled", "unknown",
])
const cleanupTakeoverStatuses = new Set([
  "not_requested", "state_unavailable", "environment_changed", "pending_cancel_seam_missing",
  "request_result_unknown", "actor_identity_unknown", "preexisting_owner_preserved",
  "no_longer_owned_by_run", "released", "release_failed", "unknown",
])

export function parseArgs(argv) {
  const options = {
    kernelUrl: null,
    sessionId: null,
    agentA: null,
    agentB: null,
    agentC: null,
    sameTabId: null,
    otherTabId: null,
    output: null,
    timeoutMs: defaultTimeoutMs,
    pollMs: minPollMs,
    execute: false,
    help: false,
  }
  const flags = new Set()
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    const value = () => {
      const next = argv[index + 1]
      assert.ok(next && !next.startsWith("--"), `missing value for ${arg}`)
      index += 1
      return next
    }
    if (["--execute", "--help", "-h"].includes(arg)) {
      assert.ok(!flags.has(arg), `duplicate flag: ${arg}`)
      flags.add(arg)
      if (arg === "--execute") options.execute = true
      else options.help = true
    } else {
      const fields = {
        "--kernel-url": "kernelUrl",
        "--session": "sessionId",
        "--agent-a": "agentA",
        "--agent-b": "agentB",
        "--agent-c": "agentC",
        "--same-tab": "sameTabId",
        "--other-tab": "otherTabId",
        "--output": "output",
        "--timeout-ms": "timeoutMs",
        "--poll-ms": "pollMs",
      }
      assert.ok(Object.hasOwn(fields, arg), `unknown argument: ${arg}`)
      assert.ok(!flags.has(arg), `duplicate argument: ${arg}`)
      flags.add(arg)
      const raw = value()
      options[fields[arg]] = fields[arg].endsWith("Ms") ? Number(raw) : raw
    }
  }
  return options
}

function assertId(value, name) {
  assert.ok(typeof value === "string" && value.length > 0 && value.length <= 256
    && value === value.trim() && !/[\u0000-\u001f\u007f]/u.test(value),
  `${name} must be a non-empty bounded identity`)
}

export function validateOptions(options) {
  for (const [field, name] of [
    ["kernelUrl", "--kernel-url"], ["sessionId", "--session"],
    ["agentA", "--agent-a"], ["agentB", "--agent-b"], ["agentC", "--agent-c"],
    ["sameTabId", "--same-tab"], ["otherTabId", "--other-tab"], ["output", "--output"],
  ]) assertId(options[field], name)
  const endpoint = new URL(options.kernelUrl)
  assert.ok(["ws:", "wss:"].includes(endpoint.protocol), "--kernel-url must be a WebSocket URL")
  assert.ok(!endpoint.username && !endpoint.password && !endpoint.search && !endpoint.hash,
    "--kernel-url must not contain credentials or opaque query data")
  assert.ok(new Set([options.agentA, options.agentB, options.agentC]).size === 3,
    "Drill E requires three distinct existing Room agent ids")
  assert.ok(options.sameTabId !== options.otherTabId, "Drill E requires two different Room tabs")
  assert.ok(path.isAbsolute(options.output), "--output must be an absolute external path")
  assert.ok(Number.isSafeInteger(options.timeoutMs)
    && options.timeoutMs >= 1_000 && options.timeoutMs <= maxTimeoutMs,
  `--timeout-ms must be between 1000 and ${maxTimeoutMs}`)
  assert.ok(Number.isSafeInteger(options.pollMs)
    && options.pollMs >= minPollMs && options.pollMs <= maxPollMs,
  `--poll-ms must be between ${minPollMs} and ${maxPollMs}`)
  return options
}

export function helpText() {
  return [
    "Usage: node apps/cli/scripts/live-browser-computer-drill-e-scenario.mjs --execute --kernel-url ws://HOST:PORT --session ID --agent-a ID --agent-b ID --agent-c ID --same-tab ID --other-tab ID --output /absolute/external/path.json [options]",
    "",
    "Opt-in driver for an existing ready Room. It submits real prompts to the three named agents and requests human takeover through LocalIpcClient.",
    "It does not create the Room, agents, tabs, pages, or provider accounts.",
    "",
    "Preconditions: same-tab is focused; both tabs contain controlled test pages; all agents are present and can use Room browser tools; reloads are slow enough to observe a running/queued pair.",
    "Provider prompt acknowledgments alone never count as drill evidence. The report is verified from kernel Environment snapshots and Action history.",
    "The drill proves only observed concurrency/arbitration in this Room; it does not certify other sessions or production readiness.",
    "Cleanup gets " + drillEScenarioCleanupBudgetMs + "ms after the observation deadline, with "
      + drillEScenarioDetachReserveMs + "ms reserved for detach. Cleanup failures prevent a passing report.",
    "",
    `  --timeout-ms N  Total observation/action bound 1000..${maxTimeoutMs} (default ${defaultTimeoutMs})`,
    `  --poll-ms N     Kernel observation interval ${minPollMs}..${maxPollMs} (default ${minPollMs})`,
    "  --execute       Required explicit opt-in before any prompts or takeover request",
  ].join("\n")
}

export function runDrillEScenario(input) {
  validateOptions(input.options)
  return runDrillEScenarioStateful(input)
}

function redactedAction(action) {
  return {
    actionId: action.action_id,
    sequence: action.sequence,
    actorId: action.actor_id,
    runtimeGeneration: action.runtime_generation,
    mode: action.mode,
    kind: action.kind,
    targets: action.targets.map((target) => target?.kind === "browser_tab"
      ? { kind: "browser_tab", id: target.id }
      : { kind: target?.kind }),
    state: action.state,
    cancellationRequested: action.cancellation_requested,
    submittedAtMs: action.submitted_at_ms,
    startedAtMs: action.started_at_ms,
    finishedAtMs: action.finished_at_ms,
    outcome: action.outcome,
  }
}

function redactedSnapshot(sample) {
  const environment = sample.environment
  return {
    observedAtMs: sample.observed_at_ms,
    sessionId: environment.session_id,
    environmentId: environment.environment_id,
    runtimeGeneration: environment.runtime_generation,
    lifecycle: environment.lifecycle,
    eventCursor: environment.event_cursor,
    focusedTabId: environment.focused_tab_id,
    actors: environment.actors.map(({ actor_id, kind, presence }) => ({ actorId: actor_id, kind, presence })),
    tabIds: environment.tabs.map(({ tab_id }) => tab_id),
    actions: environment.actions.map(redactedAction),
    pendingInputTakeovers: environment.pending_input_takeovers.map((takeover) => ({
      target: takeover.target,
      humanActorId: takeover.human_actor_id,
      blockingActionIds: takeover.blocking_action_ids,
    })),
    inputOwnership: environment.input_ownership.map((ownership) => ({
      target: ownership.target,
      actorId: ownership.actor_id,
    })),
  }
}

function evidenceDocument(capture) {
  return {
    ...capture.report,
    capturedAt: new Date().toISOString(),
    source: "LocalIpcClient RoomEnvironmentState snapshots and paged RoomEnvironmentActionHistory",
    kernelStatePollCount: capture.kernelStatePollCount,
    limitations: [
      "All four Drill E checks must pass; prompt acknowledgments do not count as kernel action evidence.",
      "This records one existing Room run and does not certify other sessions or production readiness.",
    ],
    snapshots: capture.snapshots.map(redactedSnapshot),
    actionHistory: capture.actions
      .filter((action) => action.sequence > capture.report.baselineActionSequence
        && action.runtime_generation === capture.report.runtimeGeneration)
      .map(redactedAction),
    cleanup: capture.cleanup,
  }
}

function diagnosticIdentity(value) {
  return typeof value === "string" && value.length > 0 && value.length <= 256
    && value === value.trim() && !/[\u0000-\u001f\u007f]/u.test(value)
    ? value
    : null
}

function diagnosticCleanup(cleanup) {
  if (!cleanup || typeof cleanup !== "object") {
    return {
      status: "unknown", stage: "unknown", detached: false, attachmentId: null,
      environmentId: null, runtimeGeneration: null, submittedPrompts: [], takeover: null, failures: [],
    }
  }
  const prompts = Array.isArray(cleanup.submittedPrompts) ? cleanup.submittedPrompts : []
  const takeover = cleanup.takeover && typeof cleanup.takeover === "object" ? cleanup.takeover : null
  const failures = Array.isArray(cleanup.failures) ? cleanup.failures : []
  return {
    status: ["passed", "failed"].includes(cleanup.status) ? cleanup.status : "unknown",
    stage: cleanupStages.has(cleanup.stage) ? cleanup.stage : "unknown",
    budgetMs: Number.isSafeInteger(cleanup.budgetMs) ? cleanup.budgetMs : null,
    detachReserveMs: Number.isSafeInteger(cleanup.detachReserveMs) ? cleanup.detachReserveMs : null,
    detached: cleanup.detached === true,
    attachmentId: diagnosticIdentity(cleanup.attachmentId),
    environmentId: diagnosticIdentity(cleanup.environmentId),
    runtimeGeneration: Number.isSafeInteger(cleanup.runtimeGeneration) ? cleanup.runtimeGeneration : null,
    submittedPrompts: prompts.slice(0, 16).map((prompt) => ({
      agentId: diagnosticIdentity(prompt?.agentId),
      promptId: diagnosticIdentity(prompt?.promptId),
      purpose: cleanupPromptPurposes.has(prompt?.purpose) ? prompt.purpose : null,
      admission: ["submitting", "acknowledgment_unknown", "started", "queued"].includes(prompt?.admission)
        ? prompt.admission : "unknown",
      cleanupStatus: cleanupPromptStatuses.has(prompt?.cleanupStatus) ? prompt.cleanupStatus : "unknown",
    })),
    takeover: takeover ? {
      requested: takeover.requested === true,
      targetTabId: diagnosticIdentity(takeover.targetTabId),
      previousOwnerActorId: diagnosticIdentity(takeover.previousOwnerActorId),
      humanActorId: diagnosticIdentity(takeover.humanActorId),
      pendingHumanActorId: diagnosticIdentity(takeover.pendingHumanActorId),
      blockingActionIds: Array.isArray(takeover.blockingActionIds)
        ? takeover.blockingActionIds.slice(0, 8).map(diagnosticIdentity).filter(Boolean) : [],
      environmentId: diagnosticIdentity(takeover.environmentId),
      runtimeGeneration: Number.isSafeInteger(takeover.runtimeGeneration) ? takeover.runtimeGeneration : null,
      status: cleanupTakeoverStatuses.has(takeover.status) ? takeover.status : "unknown",
    } : null,
    failures: failures.slice(0, 16).flatMap((failure) => {
      if (!cleanupFailureCodes.has(failure?.code)) return []
      return [{
        code: failure.code,
        agentId: diagnosticIdentity(failure.agentId),
        promptId: diagnosticIdentity(failure.promptId),
        purpose: cleanupPromptPurposes.has(failure.purpose) ? failure.purpose : null,
        targetTabId: diagnosticIdentity(failure.targetTabId),
      }]
    }),
  }
}

function failureDiagnosticDocument({
  error,
  options,
  stage,
  cleanup,
  checksExecuted = false,
  failureCode = null,
  capturedAt,
}) {
  const normalizedStage = diagnosticStages.has(stage) ? stage : "scenario"
  let normalizedCode = failureCode
  if (!normalizedCode) {
    if (cleanup?.failures?.some((failure) => failure?.code === "session_attachment_identity_unavailable")) {
      normalizedCode = "attachment_acknowledgment_unknown"
    } else if (typeof error?.message === "string" && /timeout|deadline/iu.test(error.message)) {
      normalizedCode = "scenario_timed_out"
    } else {
      normalizedCode = "scenario_failed"
    }
  }
  return {
    schema: "chariox.browser_computer.drill_e.scenario_diagnostic.v1",
    diagnostic: true,
    status: "failed",
    acceptanceClaimed: false,
    capturedAt,
    source: "LocalIpcClient Drill E scenario diagnostic",
    failure: { stage: normalizedStage, code: normalizedCode },
    identities: {
      sessionId: diagnosticIdentity(options.sessionId),
      agentIds: [options.agentA, options.agentB, options.agentC].map(diagnosticIdentity).filter(Boolean),
      tabIds: [options.sameTabId, options.otherTabId].map(diagnosticIdentity).filter(Boolean),
      attachmentId: diagnosticIdentity(cleanup?.attachmentId),
      environmentId: diagnosticIdentity(cleanup?.environmentId),
      runtimeGeneration: Number.isSafeInteger(cleanup?.runtimeGeneration) ? cleanup.runtimeGeneration : null,
    },
    unexecutedChecks: checksExecuted ? [] : [...drillECheckNames],
    cleanup: diagnosticCleanup(cleanup),
  }
}

async function writeEvidenceDocument(output, document) {
  const bytes = Buffer.from(`${JSON.stringify(document, null, 2)}\n`, "utf8")
  assert.ok(bytes.byteLength <= maxEvidenceBytes, "Drill E evidence exceeds its size bound")
  await output.handle.truncate(0)
  let offset = 0
  while (offset < bytes.byteLength) {
    const result = await output.handle.write(bytes, offset, bytes.byteLength - offset, offset)
    assert.ok(Number.isSafeInteger(result?.bytesWritten) && result.bytesWritten > 0,
      "Drill E evidence write made no progress")
    offset += result.bytesWritten
  }
  await output.handle.sync()
}

function isInside(root, candidate) {
  const relative = path.relative(root, candidate)
  return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative))
}

async function reserveExternalOutput(outputPath) {
  const canonicalRoot = await realpath(repoRoot)
  const absoluteOutput = path.resolve(outputPath)
  assert.ok(!isInside(canonicalRoot, absoluteOutput), "Drill E evidence must be outside the repository")
  let existingParent = path.dirname(absoluteOutput)
  const missingParts = []
  while (true) {
    try {
      await realpath(existingParent)
      break
    } catch (error) {
      if (error?.code !== "ENOENT") throw error
      missingParts.unshift(path.basename(existingParent))
      const parent = path.dirname(existingParent)
      assert.ok(parent !== existingParent, "external output parent does not exist")
      existingParent = parent
    }
  }
  const canonicalExistingParent = await realpath(existingParent)
  assert.ok(!isInside(canonicalRoot, canonicalExistingParent), "Drill E evidence must be outside the repository")
  const canonicalParent = path.join(canonicalExistingParent, ...missingParts)
  assert.ok(!isInside(canonicalRoot, canonicalParent), "Drill E evidence must be outside the repository")
  await mkdir(canonicalParent, { recursive: true, mode: 0o700 })
  const verifiedParent = await realpath(canonicalParent)
  assert.ok(!isInside(canonicalRoot, verifiedParent), "Drill E evidence must be outside the repository")
  const canonicalOutput = path.join(verifiedParent, path.basename(absoluteOutput))
  const handle = await open(canonicalOutput, "wx", 0o600)
  await handle.chmod(0o600)
  return { handle, outputPath: canonicalOutput }
}

export async function runDrillEScenarioCli({
  options,
  client = null,
  requests = null,
  loadRuntime = null,
  runScenario = runDrillEScenario,
  reserveOutput = reserveExternalOutput,
  logger = console,
  capturedAt = () => new Date().toISOString(),
}) {
  validateOptions(options)
  assert.ok(options.execute, "refusing to prompt agents without explicit --execute")
  const output = await reserveOutput(options.output)
  try {
    let capture
    let stage = "runtime_setup"
    try {
      if (loadRuntime) {
        const runtime = await loadRuntime()
        client = runtime.client
        requests = runtime.requests
      }
      stage = "scenario"
      capture = await runScenario({ client, requests, options })
    } catch (error) {
      const diagnostic = failureDiagnosticDocument({
        error,
        options,
        stage: error?.drillEStage ?? stage,
        cleanup: error?.cleanup,
        capturedAt: capturedAt(),
      })
      try {
        await writeEvidenceDocument(output, diagnostic)
      } catch {
        logger.error("[drill-e-scenario] failed diagnostic write; reserved output retained")
        throw new Error("Drill E failure diagnostic could not be persisted")
      }
      const cleanupCodes = diagnostic.cleanup.failures.map((failure) => failure.code).join(",")
      logger.error(`[drill-e-scenario] failed at ${diagnostic.failure.stage} (${diagnostic.failure.code}); diagnostic: ${output.outputPath}; cleanup: ${diagnostic.cleanup.status}${cleanupCodes ? "; cleanup failures: " + cleanupCodes : ""}`)
      return { status: "failed", diagnostic: true, exitCode: 1, outputPath: output.outputPath }
    }

    let document
    try {
      document = evidenceDocument(capture)
      await writeEvidenceDocument(output, document)
    } catch (error) {
      const failureCode = error?.message === "Drill E evidence exceeds its size bound"
        ? "evidence_size_limit_exceeded"
        : "evidence_write_failed"
      const diagnostic = failureDiagnosticDocument({
        error,
        options,
        stage: "evidence_write",
        cleanup: capture.cleanup,
        checksExecuted: true,
        failureCode,
        capturedAt: capturedAt(),
      })
      try {
        await writeEvidenceDocument(output, diagnostic)
      } catch {
        logger.error("[drill-e-scenario] failed diagnostic write; reserved output retained")
        throw new Error("Drill E failure diagnostic could not be persisted")
      }
      const cleanupCodes = diagnostic.cleanup.failures.map((failure) => failure.code).join(",")
      logger.error(`[drill-e-scenario] failed at ${diagnostic.failure.stage} (${diagnostic.failure.code}); diagnostic: ${output.outputPath}; cleanup: ${diagnostic.cleanup.status}${cleanupCodes ? "; cleanup failures: " + cleanupCodes : ""}`)
      return { status: "failed", diagnostic: true, exitCode: 1, outputPath: output.outputPath }
    }

    logger.log("[drill-e-scenario] cleanup: " + capture.cleanup.status
      + "; detached: " + capture.cleanup.detached)
    logger.log(`[drill-e-scenario] ${capture.report.status}; report: ${output.outputPath}`)
    for (const [name, check] of Object.entries(capture.report.checks)) {
      logger.log(`[drill-e-scenario] ${name}: ${check.status}${check.reason ? ` — ${check.reason}` : ""}`)
    }
    const exitCode = capture.report.status === "passed" ? 0 : 2
    return { status: capture.report.status, diagnostic: false, exitCode, outputPath: output.outputPath }
  } finally {
    try {
      await client?.close?.()
    } catch {
      // Preserve the already-fsynced evidence if transport close fails.
    }
    await output.handle.close().catch(() => {})
  }
}

async function runCli() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    console.log(helpText())
    return
  }
  const result = await runDrillEScenarioCli({
    options,
    loadRuntime: async () => {
      const kernelClientUrl = pathToFileURL(path.resolve(repoRoot, "packages/kernel-client/dist/ipc.js")).href
      const requestsUrl = pathToFileURL(path.resolve(repoRoot, "packages/kernel-client/dist/ipc-requests.js")).href
      const [{ LocalIpcClient }, requests] = await Promise.all([import(kernelClientUrl), import(requestsUrl)])
      return { client: new LocalIpcClient(options.kernelUrl), requests }
    },
  })
  process.exitCode = result.exitCode
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  runCli().catch(() => {
    console.error("[drill-e-scenario] failed closed; reserved output was retained")
    process.exitCode = 1
  })
}
