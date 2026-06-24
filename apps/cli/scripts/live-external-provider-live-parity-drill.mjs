#!/usr/bin/env node
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { readFileSync } from "node:fs"
import { mkdir, readFile, readdir, rm, stat, writeFile } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { pathToFileURL } from "node:url"
import { setTimeout as sleep } from "node:timers/promises"

import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  getSessionHistoryBlobContentRequest,
  getSessionHistoryOutlineRequest,
  getSessionStateRequest,
  importExternalProviderSessionRequest,
  listExternalProviderSessionsRequest,
} from "../dist/ipc-requests.js"

const scriptDir = path.dirname(new URL(import.meta.url).pathname)
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cloudRepo = process.env.ARROBA_CLOUD_REPO ?? path.resolve(repoRoot, "..", "arroba-cloud")
const defaultKernelUrl = process.env.ARROBA_EXTERNAL_PARITY_KERNEL_URL ?? "ws://127.0.0.1:44120/kernel"
const defaultWebUrl = process.env.ARROBA_EXTERNAL_PARITY_WEB_URL ?? "http://127.0.0.1:4321"
const defaultProviders = ["codex", "claude", "opencode"]
const defaultModels = {
  codex: process.env.ARROBA_EXTERNAL_PARITY_CODEX_MODEL ?? "gpt-5.5",
  claude: process.env.ARROBA_EXTERNAL_PARITY_CLAUDE_MODEL ?? "sonnet",
  opencode: process.env.ARROBA_EXTERNAL_PARITY_OPENCODE_MODEL ?? "opencode/kimi-k2.6",
}
const requiredAssistantMarkers = Array.from({ length: 20 }, (_, index) => `ASSISTANT_STEP_${String(index + 1).padStart(2, "0")}`)
const requiredToolMarkers = Array.from({ length: 20 }, (_, index) => `TOOL_STEP_${String(index + 1).padStart(2, "0")}`)
const finalMarkerPrefix = "FINAL_EXTERNAL_PARITY_SUMMARY"

function parseArgs(argv) {
  const options = {
    providers: [...defaultProviders],
    providerModels: new Map(),
    kernelUrl: defaultKernelUrl,
    webUrl: defaultWebUrl,
    workspace: repoRoot,
    artifactRoot: path.join(repoRoot, ".artifacts", "external-provider-live-parity", nowStamp()),
    timeoutMs: 900_000,
    pollMs: 1_000,
    dryRun: false,
    skipWeb: false,
    skipTui: false,
    keepArtifactsOnSuccess: true,
  }
  for (let index = 2; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") {
      continue
    } else if (arg === "--providers" || arg === "--provider") {
      options.providers = readValue(argv, ++index, arg).split(",").map((provider) => provider.trim()).filter(Boolean)
    } else if (arg === "--provider-model") {
      const value = readValue(argv, ++index, arg)
      const [provider, model] = value.split("=", 2)
      if (!provider || !model) throw new Error("--provider-model must be PROVIDER=MODEL")
      options.providerModels.set(provider.trim(), model.trim())
    } else if (arg === "--kernel-url") {
      options.kernelUrl = readValue(argv, ++index, arg)
    } else if (arg === "--web-url") {
      options.webUrl = readValue(argv, ++index, arg)
    } else if (arg === "--workspace") {
      options.workspace = path.resolve(readValue(argv, ++index, arg))
    } else if (arg === "--artifact-root") {
      options.artifactRoot = path.resolve(readValue(argv, ++index, arg))
    } else if (arg === "--timeout-ms") {
      options.timeoutMs = Number(readValue(argv, ++index, arg))
    } else if (arg === "--poll-ms") {
      options.pollMs = Number(readValue(argv, ++index, arg))
    } else if (arg === "--dry-run") {
      options.dryRun = true
    } else if (arg === "--skip-web") {
      options.skipWeb = true
    } else if (arg === "--skip-tui") {
      options.skipTui = true
    } else if (arg === "--keep-artifacts-on-success") {
      options.keepArtifactsOnSuccess = true
    } else if (arg === "--help" || arg === "-h") {
      printHelp()
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  if (options.providers.length === 0) throw new Error("at least one provider is required")
  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) throw new Error("--timeout-ms must be positive")
  if (!Number.isFinite(options.pollMs) || options.pollMs <= 0) throw new Error("--poll-ms must be positive")
  return options
}

function readValue(argv, index, flag) {
  const value = argv[index]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}

function printHelp() {
  console.log(`Usage: node apps/cli/scripts/live-external-provider-live-parity-drill.mjs [options]

Runs live external-provider parity drills for Codex, Claude, and OpenCode.

Options:
  --providers codex,claude,opencode
  --provider-model codex=gpt-5.5
  --provider-model claude=sonnet
  --provider-model opencode=opencode/kimi-k2.6
  --kernel-url ws://127.0.0.1:44120/kernel
  --web-url http://127.0.0.1:4321
  --workspace /path/to/workspace
  --artifact-root .artifacts/external-provider-live-parity/<stamp>
  --timeout-ms 900000
  --poll-ms 1000
  --dry-run
  --skip-web
  --skip-tui
  --keep-artifacts-on-success  Preserve artifacts after successful runs (default)

Provider command overrides:
  ARROBA_EXTERNAL_PARITY_CODEX_COMMAND='codex exec --model {model} {prompt}'
  ARROBA_EXTERNAL_PARITY_CLAUDE_COMMAND='claude -p --model {model} {prompt}'
  ARROBA_EXTERNAL_PARITY_OPENCODE_COMMAND='opencode run -m {model} {prompt}'
`)
}

function nowStamp() {
  return new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z")
}

function unwrap(response, variant) {
  if (!response || !(variant in response)) {
    throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  }
  return response[variant]
}

function providerModel(options, provider) {
  return options.providerModels.get(provider) ?? defaultModels[provider] ?? "default"
}

function buildPrompt(provider, marker, workspace, observerGate) {
  const promptMarker = `EXTERNAL_PARITY_USER_PROMPT_${marker}`
  const text = [
    `You are running the Arroba external provider live parity drill for provider ${provider}.`,
    `Drill marker: ${marker}.`,
    `User prompt marker: ${promptMarker}.`,
    `Workspace: ${workspace}.`,
    `Scratch file prefix: ${observerGate.scratchFilePrefix}.`,
    `Observer gate go file: ${observerGate.goFile}.`,
    "",
    "Requirements:",
    "1. Before emitting any ASSISTANT_STEP_NN, TOOL_STEP_NN, or FINAL_EXTERNAL_PARITY_SUMMARY marker, wait until the observer gate go file exists.",
    "2. The observer gate setup/wait can use provider tools, but it must not write files, must not include TOOL_STEP_NN markers, and it does not count toward the 20 marked tool calls.",
    "3. After the go file exists, produce exactly 20 separate assistant progress messages, each with an assistant step marker.",
    "4. Assistant step markers must be built from prefix ASSISTANT_STEP_ plus two-digit numbers from 01 to 20.",
    "5. After the go file exists, run exactly 20 observable marked tool calls, each with a tool step marker.",
    "6. Tool step markers must be built from prefix TOOL_STEP_ plus two-digit numbers from 01 to 20.",
    "7. Each assistant message must include its ASSISTANT_STEP_NN marker and the drill marker.",
    "8. Each marked tool call must make its TOOL_STEP_NN marker observable in either the command, path, or output.",
    "9. Use only workspace-root temporary drill files whose paths begin with the scratch file prefix.",
    "10. Create, append, read, list, inspect metadata, and delete small text files with that scratch file prefix.",
    "11. Delete the temporary scratch files before finishing, but do not delete the observer gate go file.",
    `12. End with the exact final summary marker formed by joining prefix ${finalMarkerPrefix}, an underscore, and the drill marker.`,
    "13. Do not repeat the user prompt marker in assistant progress messages, tool command text, tool output, or the final summary.",
    "14. Use only low-risk shell commands: printf, cat, ls, wc, stat, find, test, touch, rm, rmdir, and sleep.",
    "15. Do not use xattr, chmod, chown, install, Python, Node, Ruby, Perl, network commands, package managers, git, or any command likely to require interactive approval.",
    "16. Include a short `sleep 0.2` in each marked tool call so Arroba can observe the live turn before the final summary.",
  ].join("\n")
  return { text, promptMarker }
}

function finalMarkerFor(marker) {
  return `${finalMarkerPrefix}_${marker}`
}

function observerGate(marker, workspace) {
  const dir = workspace
  const scratchFilePrefix = path.join(workspace, `external-provider-live-parity-drill-${marker}`)
  return {
    dir,
    scratchFilePrefix,
    goFile: path.join(workspace, `external-provider-live-parity-go-${marker}.txt`),
  }
}

function providerCommand(provider, model, prompt, workspace) {
  const override = process.env[`ARROBA_EXTERNAL_PARITY_${provider.toUpperCase()}_COMMAND`]
  if (override?.trim()) {
    return {
      command: "sh",
      args: ["-lc", templateCommand(override, { provider, model, prompt, workspace })],
      shellTemplate: override,
    }
  }
  if (provider === "codex") {
    return { command: "codex", args: ["exec", "--model", model, prompt] }
  }
  if (provider === "claude") {
    return { command: "claude", args: ["-p", "--model", model, "--permission-mode", "bypassPermissions", "--dangerously-skip-permissions", prompt] }
  }
  if (provider === "opencode") {
    return { command: "opencode", args: ["run", "-m", model, prompt] }
  }
  throw new Error(`unsupported provider ${provider}`)
}

function templateCommand(template, values) {
  return template.replace(/\{(provider|model|prompt|workspace)\}/g, (_, key) => shellQuote(values[key]))
}

function shellQuote(value) {
  return `'${String(value).replace(/'/g, `'\\''`)}'`
}

async function main() {
  const options = parseArgs(process.argv)
  await prepareDrillArtifacts(options.artifactRoot)
  const summary = {
    ok: false,
    drill: "external-provider-live-parity",
    createdAt: new Date().toISOString(),
    artifactRoot: options.artifactRoot,
    kernelUrl: options.kernelUrl,
    webUrl: options.webUrl,
    workspace: options.workspace,
    dryRun: options.dryRun,
    providers: options.providers,
    results: [],
    providerLimitations: [],
  }
  let failure = null
  try {
    for (const provider of options.providers) {
      const result = await runProviderDrill(provider, options)
      summary.results.push(result)
      if (!result.ok) break
    }
    summary.ok = summary.results.length === options.providers.length && summary.results.every((result) => result.ok)
    summary.providerLimitations = collectProviderLimitations(summary.results)
    await writeJson(path.join(options.artifactRoot, "manifest.json"), summary)
    await writeFinalReport(options.artifactRoot, summary)
    if (!summary.ok) {
      throw new Error(`external provider live parity drill failed: ${summary.results.filter((result) => !result.ok).map((result) => result.provider).join(", ")}`)
    }
  } catch (error) {
    failure = error
    summary.ok = false
    summary.error = String(error?.stack ?? error)
    summary.providerLimitations = collectProviderLimitations(summary.results)
    await writeJson(path.join(options.artifactRoot, "manifest.json"), summary).catch(() => {})
    await writeFinalReport(options.artifactRoot, summary).catch(() => {})
    throw error
  } finally {
    await finalizeDrillArtifacts({
      rootDir: options.artifactRoot,
      passed: summary.ok,
      preserveOnFailure: true,
      preserveOnSuccess: options.keepArtifactsOnSuccess || options.dryRun,
      failure,
      metadata: {
        drill: "external-provider-live-parity",
        providers: options.providers,
        kernelUrl: options.kernelUrl,
        webUrl: options.webUrl,
      },
      log: (message, details) => {
        if (details === undefined) console.log(`[external-live-parity] ${message}`)
        else console.log(`[external-live-parity] ${message}`, JSON.stringify(details))
      },
    })
  }
  console.log(JSON.stringify(summary, null, 2))
}

function collectProviderLimitations(results) {
  return results.flatMap((result) => result.providerLimitations ?? [])
}

async function runProviderDrill(provider, options) {
  const startedAt = Date.now()
  const marker = `ARROBA_EXTERNAL_PARITY_${provider.toUpperCase()}_${process.pid}_${Date.now()}`
  const model = providerModel(options, provider)
  const providerRoot = path.join(options.artifactRoot, provider)
  const gate = observerGate(marker, options.workspace)
  const finalMarker = finalMarkerFor(marker)
  const prompt = buildPrompt(provider, marker, options.workspace, gate)
  const command = providerCommand(provider, model, prompt.text, options.workspace)
  await mkdir(providerRoot, { recursive: true })
  await writeFile(path.join(providerRoot, "prompt.txt"), prompt.text, "utf8")
  await writeJson(path.join(providerRoot, "provider-command.json"), {
    provider,
    model,
    command: command.command,
    args: command.args,
    shellTemplate: command.shellTemplate ?? null,
  })

  const result = {
    ok: false,
    provider,
    model,
    marker,
    finalMarker,
    promptMarker: prompt.promptMarker,
    artifactDir: providerRoot,
    durationMs: 0,
    externalSessionId: null,
    providerSessionId: null,
    arrobaSessionId: null,
    agentId: null,
    assertions: [],
    providerLimitations: [],
    evidence: {},
  }
  result.evidence.observerGate = gate

  if (options.dryRun) {
    result.ok = true
    result.durationMs = Date.now() - startedAt
    result.assertions.push(pass("dry run generated provider prompt and command"))
    result.providerLimitations.push({
      provider,
      surface: "all",
      status: "skipped",
      classification: "dry_run",
      note: "dry run does not validate provider, web, or TUI behavior",
    })
    await writeJson(path.join(providerRoot, "manifest.json"), result)
    return result
  }

  let client = null
  let providerProcess = null
  let tuiProcess = null
  let tuiSocketPath = null
  let browser = null
  let context = null
  let page = null
  try {
    client = new LocalIpcClient(options.kernelUrl)
    await client.send({ RefreshExternalProviderSessions: { provider } }).catch(() => null)
    const before = await listExternalProviderSessions(client, provider)
    providerProcess = spawnProviderProcess(command, providerRoot, options.workspace)

    const external = await waitForNewExternalSession({
      client,
      provider,
      before,
      marker,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
    })
    result.externalSessionId = external.external_session_id
    result.providerSessionId = external.provider_session_id ?? null
    result.assertions.push(pass("external provider session appeared in Arroba unattached inventory"))

    const imported = unwrap(
      await client.send(importExternalProviderSessionRequest(external.external_session_id, {
        alias: `${provider}-external-live-${marker.slice(-8).toLowerCase()}`,
        provider,
        model,
      })),
      "ExternalProviderSessionImported",
    )
    result.arrobaSessionId = imported.session.id
    result.agentId = imported.agent.id
    result.assertions.push(pass("external provider session imported into Arroba session"))

    const monitors = []
    monitors.push(startKernelMonitor({ client, sessionId: result.arrobaSessionId, agentId: result.agentId, provider, marker, finalMarker, promptMarker: prompt.promptMarker, options }))
    if (!options.skipTui) {
      const tui = await startTuiObserver({ sessionId: result.arrobaSessionId, options, providerRoot })
      tuiProcess = tui.process
      tuiSocketPath = tui.socketPath
      monitors.push(startTuiMonitor({ socketPath: tui.socketPath, provider, marker, finalMarker, promptMarker: prompt.promptMarker, options }))
      result.evidence.tuiSocketPath = tui.socketPath
    }
    if (!options.skipWeb) {
      const web = await startWebObserver({ sessionId: result.arrobaSessionId, webUrl: options.webUrl, providerRoot })
      browser = web.browser
      context = web.context
      page = web.page
      monitors.push(startWebMonitor({ page, provider, marker, finalMarker, promptMarker: prompt.promptMarker, providerRoot, options }))
    }

    await releaseObserverGate(gate, marker)
    result.assertions.push(pass("observer gate released after Arroba monitors started"))

    const providerExit = await waitForProviderExit(providerProcess, options.timeoutMs)
    result.evidence.providerExit = providerExit
    await waitForKernelFinalIdle({ client, sessionId: result.arrobaSessionId, agentId: result.agentId, provider, marker, finalMarker, promptMarker: prompt.promptMarker, options })
    if (page) {
      await waitForSurfaceFinalIdle({
        surface: "web",
        sample: () => webSample(page, provider, marker, finalMarker, prompt.promptMarker),
        options,
      })
      await expandWebTranscript(page, options)
    }
    if (tuiSocketPath) {
      await waitForSurfaceFinalIdle({
        surface: "tui",
        sample: () => tuiSample(tuiSocketPath, provider, marker, finalMarker, prompt.promptMarker),
        options,
      })
      await expandTuiTranscriptBlobs(tuiSocketPath, options)
      await waitForSurfaceFinalIdle({
        surface: "tui",
        sample: () => tuiSample(tuiSocketPath, provider, marker, finalMarker, prompt.promptMarker),
        options,
      })
    }
    const providerTranscript = await snapshotProviderTranscript({
      provider,
      providerSessionId: result.providerSessionId,
      providerRoot,
      finalMarker,
      promptMarker: prompt.promptMarker,
    })
    result.evidence.providerTranscript = providerTranscript
    const monitorResults = []
    for (const monitor of monitors) {
      monitorResults.push(await monitor.stop())
    }
    result.evidence.monitors = monitorResults
    const kernel = monitorResults.find((entry) => entry.surface === "kernel")
    const web = monitorResults.find((entry) => entry.surface === "web")
    const tui = monitorResults.find((entry) => entry.surface === "tui")

    assertProviderTranscript(result, providerTranscript, "provider transcript")
    assertSurface(result, kernel, "kernel history")
    if (!options.skipWeb) assertSurface(result, web, "product web terminal")
    if (!options.skipTui) assertSurface(result, tui, "TUI")
    assertLiveObservation(result, kernel, "kernel")
    if (web) assertLiveObservation(result, web, "web")
    if (tui) assertLiveObservation(result, tui, "tui")
    assertBadgeLifecycle(result, kernel, "kernel")
    if (web) assertBadgeLifecycle(result, web, "web")
    if (tui) assertBadgeLifecycle(result, tui, "tui")
    if (web) assertWebTurnCollapse(result, web)

    result.providerLimitations = providerLimitations(provider, { kernel, web, tui, providerTranscript }, {
      providerSessionId: result.providerSessionId,
      externalSessionId: result.externalSessionId,
      arrobaSessionId: result.arrobaSessionId,
      agentId: result.agentId,
      model,
      providerExit,
    })
    if (!providerTranscript.found) {
      result.providerLimitations.push({
        provider,
        surface: "provider_transcript",
        status: "not_observed",
        classification: "drill_observation_limitation",
        note: providerTranscript.reason ?? "provider-native transcript file was not found",
      })
    }
    result.ok = result.assertions.every((assertion) => assertion.passed)
  } catch (error) {
    result.ok = false
    result.error = String(error?.stack ?? error)
    result.assertions.push(fail("provider drill completed without exception", result.error))
  } finally {
    await closeWithTimeout(context, "browser context")
    await closeWithTimeout(browser, "browser")
    stopChild(tuiProcess)
    stopChild(providerProcess)
    await cleanupObserverGate(gate)
    client?.close?.()
    result.durationMs = Date.now() - startedAt
    await writeJson(path.join(providerRoot, "manifest.json"), result)
  }
  return result
}

async function releaseObserverGate(gate, marker) {
  await mkdir(gate.dir, { recursive: true })
  await writeFile(gate.goFile, `arroba observers attached for ${marker}\n`, "utf8")
}

async function cleanupObserverGate(gate) {
  await rm(gate.goFile, { force: true }).catch(() => {})
  const scratchPrefix = path.basename(gate.scratchFilePrefix ?? "")
  if (!scratchPrefix) return
  const entries = await readdir(gate.dir, { withFileTypes: true }).catch(() => [])
  await Promise.all(entries.map(async (entry) => {
    if (!entry.name.startsWith(scratchPrefix)) return
    await rm(path.join(gate.dir, entry.name), { recursive: true, force: true }).catch(() => {})
  }))
}

function spawnProviderProcess(command, artifactDir, workspace) {
  const stdoutPath = path.join(artifactDir, "provider.stdout.log")
  const stderrPath = path.join(artifactDir, "provider.stderr.log")
  const child = spawn(command.command, command.args, {
    cwd: workspace,
    env: process.env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  let stdout = ""
  let stderr = ""
  child.stdout.on("data", (chunk) => {
    stdout += chunk.toString("utf8")
    if (stdout.length > 250_000) stdout = stdout.slice(-250_000)
    void writeFile(stdoutPath, stdout, "utf8").catch(() => {})
  })
  child.stderr.on("data", (chunk) => {
    stderr += chunk.toString("utf8")
    if (stderr.length > 250_000) stderr = stderr.slice(-250_000)
    void writeFile(stderrPath, stderr, "utf8").catch(() => {})
  })
  child.once("exit", () => {
    void writeFile(stdoutPath, stdout, "utf8").catch(() => {})
    void writeFile(stderrPath, stderr, "utf8").catch(() => {})
  })
  return child
}

async function waitForProviderExit(child, timeoutMs) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return { code: child.exitCode, signal: child.signalCode }
  }
  return await Promise.race([
    new Promise((resolve) => {
      child.once("exit", (code, signal) => resolve({ code, signal }))
      child.once("error", (error) => resolve({ code: null, signal: null, error: String(error?.stack ?? error) }))
    }),
    sleep(timeoutMs).then(() => {
      stopChild(child)
      return { code: null, signal: "timeout" }
    }),
  ])
}

async function listExternalProviderSessions(client, provider) {
  const response = unwrap(
    await client.send(listExternalProviderSessionsRequest({ provider, limit: 100 })),
    "ExternalProviderSessionsListed",
  )
  return response.page.sessions ?? []
}

async function waitForNewExternalSession({ client, provider, before, marker, timeoutMs, pollMs }) {
  const beforeIds = new Set(before.map((session) => session.external_session_id))
  const deadline = Date.now() + timeoutMs
  let last = []
  while (Date.now() < deadline) {
    await client.send({ RefreshExternalProviderSessions: { provider } }).catch(() => null)
    last = await listExternalProviderSessions(client, provider)
    const candidates = last.filter((session) => !beforeIds.has(session.external_session_id))
    const marked = candidates.find((session) => JSON.stringify(session).includes(marker))
    if (marked) return marked
    if (candidates.length === 1) return candidates[0]
    if (candidates.length > 1) {
      candidates.sort((left, right) => String(right.last_modified_at ?? "").localeCompare(String(left.last_modified_at ?? "")))
      return candidates[0]
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for new ${provider} external session; last=${JSON.stringify(last.slice(0, 5), null, 2)}`)
}

function startKernelMonitor({ client, sessionId, agentId, provider, marker, finalMarker, promptMarker, options }) {
  const samples = []
  let stopped = false
  const loop = (async () => {
    while (!stopped) {
      samples.push(await kernelSample({ client, sessionId, agentId, provider, marker, finalMarker, promptMarker }).catch((error) => ({
        at: new Date().toISOString(),
        error: String(error?.message ?? error),
      })))
      await sleep(options.pollMs)
    }
  })()
  return {
    async stop() {
      stopped = true
      await loop.catch(() => {})
      const finalSample = await kernelSample({ client, sessionId, agentId, provider, marker, finalMarker, promptMarker }).catch((error) => ({ error: String(error?.message ?? error) }))
      samples.push(finalSample)
      return summarizeSamples("kernel", samples, finalMarker)
    },
  }
}

async function kernelSample({ client, sessionId, agentId, provider, marker, finalMarker, promptMarker }) {
  const stateResponse = await client.send(getSessionStateRequest(sessionId))
  const sessionState = stateResponse.SessionState ?? stateResponse.SessionStateLoaded ?? {}
  const session = sessionState.session
  const agent = (session?.agents ?? []).find((entry) => entry.id === agentId)
  const agentActivity = sessionState.agent_activity?.[agentId]
    ?? sessionState.agentActivity?.[agentId]
    ?? sessionState.agent_activity?.[String(agentId)]
    ?? null
  const outline = unwrap(await client.send(getSessionHistoryOutlineRequest(sessionId, [agentId], 1)), "SessionHistoryOutline")
  const text = await historyOutlineTextWithBlobContent({ client, sessionId, agentId, outline })
  return {
    at: new Date().toISOString(),
    surface: "kernel",
    status: agentStatus(agent, agentActivity),
    text,
    assistantMarkers: requiredAssistantMarkers.filter((entry) => text.includes(entry)),
    toolMarkers: requiredToolMarkers.filter((entry) => text.includes(entry)),
    finalSeen: text.includes(finalMarker),
    promptOccurrences: countOccurrences(text, promptMarker),
    provider,
  }
}

async function historyOutlineTextWithBlobContent({ client, sessionId, agentId, outline }) {
  const chunks = []
  for (const agent of outline.agents ?? []) {
    for (const turn of agent.turns ?? []) {
      if (turn.user_prompt?.entry?.text) chunks.push(turn.user_prompt.entry.text)
      const items = [
        ...(turn.entries ?? []).map((entry) => ({ sequence: entry.entry_index ?? 0, entry })),
        ...(turn.blobs ?? []).map((blob) => ({ sequence: blob.sequence_start ?? 0, blob })),
        ...(turn.summary ? [{ sequence: turn.summary.entry_index ?? Number.MAX_SAFE_INTEGER, entry: turn.summary }] : []),
      ].sort((left, right) => left.sequence - right.sequence)
      for (const item of items) {
        if ("entry" in item) {
          if (item.entry?.entry?.text) chunks.push(item.entry.entry.text)
          continue
        }
        const blob = item.blob
        if (!blob?.blob_id) {
          if (blob?.summary) chunks.push(blob.summary)
          continue
        }
        const blobText = await loadHistoryBlobText(client, sessionId, agent.agent_id ?? agentId, blob.blob_id)
          .catch(() => blob.summary ?? "")
        if (blobText) chunks.push(blobText)
      }
    }
  }
  return chunks.join("\n")
}

async function loadHistoryBlobText(client, sessionId, agentId, blobId) {
  const response = unwrap(
    await client.send(getSessionHistoryBlobContentRequest(sessionId, agentId, blobId)),
    "SessionHistoryBlobContent",
  )
  return (response.entries ?? [])
    .map((entry) => entry.entry?.text ?? "")
    .join("\n")
}

async function waitForKernelFinalIdle({ client, sessionId, agentId, provider, marker, finalMarker, promptMarker, options }) {
  const deadline = Date.now() + Math.min(120_000, Math.max(15_000, options.timeoutMs))
  let lastSample = null
  while (Date.now() < deadline) {
    lastSample = await kernelSample({ client, sessionId, agentId, provider, marker, finalMarker, promptMarker }).catch((error) => ({
      error: String(error?.message ?? error),
    }))
    if (!lastSample.error && lastSample.finalSeen && String(lastSample.status).toUpperCase() !== "WORKING") {
      await sleep(Math.max(750, options.pollMs))
      return lastSample
    }
    await sleep(options.pollMs)
  }
  throw new Error(`external turn did not settle to final idle in kernel history; last=${JSON.stringify(lastSample)}`)
}

async function waitForSurfaceFinalIdle({ surface, sample, options }) {
  const deadline = Date.now() + Math.min(45_000, Math.max(10_000, options.timeoutMs))
  let lastSample = null
  let stableIdleCount = 0
  while (Date.now() < deadline) {
    lastSample = await sample().catch((error) => ({
      error: String(error?.message ?? error),
    }))
    const status = normalizeLifecycleStatus(lastSample.status)
    if (!lastSample.error && lastSample.finalSeen && status !== "WORKING") {
      stableIdleCount += 1
      if (stableIdleCount >= 2) {
        await sleep(Math.max(750, options.pollMs))
        return lastSample
      }
    } else {
      stableIdleCount = 0
    }
    await sleep(options.pollMs)
  }
  throw new Error(`${surface} did not observe final idle; last=${JSON.stringify(lastSample)}`)
}

function agentStatus(agent, agentActivity = null) {
  if (agentActivity) {
    const status = String(agentActivity.status ?? "").toLowerCase()
    const promptStatus = String(agentActivity.prompt_status ?? agentActivity.promptStatus ?? "").toLowerCase()
    if (
      agentActivity.busy === true
      || agentActivity.active_turn
      || agentActivity.activeTurn
      || ["working", "running", "thinking", "streaming"].includes(status)
      || (promptStatus && promptStatus !== "none" && promptStatus !== "idle")
    ) {
      return "WORKING"
    }
    if (status === "error") return "ERROR"
    return "IDLE"
  }
  if (!agent) return "UNKNOWN"
  if (agent.is_processing || String(agent.state ?? "").toLowerCase() === "working") return "WORKING"
  return "IDLE"
}

async function startTuiObserver({ sessionId, options, providerRoot }) {
  const socketPath = path.join(os.tmpdir(), `arroba-external-parity-${process.pid}-${sessionId}.sock`)
  const stdoutPath = path.join(providerRoot, "tui.stdout.log")
  const stderrPath = path.join(providerRoot, "tui.stderr.log")
  await rm(socketPath, { force: true }).catch(() => {})
  const child = spawn("bun", [
    path.join(cliRoot, "dist/index.js"),
    "--kernel-url",
    options.kernelUrl,
    "--session",
    sessionId,
    "--automation-socket",
    socketPath,
  ], {
    cwd: options.workspace,
    env: process.env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  pipeChildLogs(child, stdoutPath, stderrPath)
  await waitForAutomation(socketPath, child)
  return { process: child, socketPath }
}

function startTuiMonitor({ socketPath, provider, marker, finalMarker, promptMarker, options }) {
  const samples = []
  let stopped = false
  const loop = (async () => {
    while (!stopped) {
      samples.push(await tuiSample(socketPath, provider, marker, finalMarker, promptMarker).catch((error) => ({
        at: new Date().toISOString(),
        error: String(error?.message ?? error),
      })))
      await sleep(options.pollMs)
    }
  })()
  return {
    async stop() {
      stopped = true
      await loop.catch(() => {})
      samples.push(await tuiSample(socketPath, provider, marker, finalMarker, promptMarker).catch((error) => ({ error: String(error?.message ?? error) })))
      return summarizeSamples("tui", samples, finalMarker)
    },
  }
}

async function tuiSample(socketPath, provider, marker, finalMarker, promptMarker) {
  const snapshot = await automationRequest(socketPath, { action: "snapshot" })
  const transcriptEntries = (snapshot.transcript?.entries ?? []).filter(Boolean)
  const paneEntries = Object.values(snapshot.agentPanes ?? {})
    .flat()
    .filter(Boolean)
  const transcriptText = transcriptEntries
    .map((entry) => entry.text ?? "")
    .join("\n")
  const paneText = paneEntries
    .map((entry) => entry.text ?? "")
    .join("\n")
  const text = [transcriptText, paneText].filter(Boolean).join("\n")
  const badge = snapshot.session?.agents?.[0]?.badge ?? null
  const entries = [...transcriptEntries, ...paneEntries]
  return {
    at: new Date().toISOString(),
    surface: "tui",
    status: badge?.label ?? badge?.tone ?? "UNKNOWN",
    text,
    assistantMarkers: requiredAssistantMarkers.filter((entry) => text.includes(entry)),
    toolMarkers: requiredToolMarkers.filter((entry) => text.includes(entry)),
    finalSeen: text.includes(finalMarker),
    promptOccurrences: Math.max(
      countOccurrences(transcriptText, promptMarker),
      countOccurrences(paneText, promptMarker),
    ),
    collapsedEntries: entries.filter((entry) => entry.blobCollapsed === true).length,
    expandedEntries: entries.filter((entry) => entry.blobCollapsed === false).length,
    provider,
  }
}

async function startWebObserver({ sessionId, webUrl, providerRoot }) {
  const { launchChromiumBrowser } = await import(pathToFileURL(path.join(cloudRepo, "scripts/lib/playwright.mjs")).href)
  const browser = await launchChromiumBrowser({ headless: process.env.ARROBA_EXTERNAL_PARITY_WEB_HEADFUL !== "1" })
  const context = await browser.newContext({ baseURL: webUrl, viewport: { width: 1500, height: 980 } })
  const page = await context.newPage()
  page.on("pageerror", (error) => console.log(`[web-pageerror] ${error.message}`))
  await page.goto("/waiting-room", { waitUntil: "domcontentloaded" })
  await waitForProductKernelReady(page, 90_000)
  await waitForWaitingRoomSessionRow(page, sessionId, 90_000)
  await waitForWaitingRoomSessionRowEnabled(page, sessionId, 30_000)
  await captureWebScreenshot(page, path.join(providerRoot, "web-waiting-room.png"))
  await openSessionFromWaitingRoom(page, sessionId)
  await page.locator("[data-freeform-pane-grid], .freeform-workspace").first().waitFor({ timeout: 90_000 })
  await clearSessionPickerOverlay(page)
  await captureWebScreenshot(page, path.join(providerRoot, "web-opened.png"))
  return { browser, context, page }
}

async function waitForProductKernelReady(page, timeoutMs) {
  await waitForWebCondition(page, async () => {
    const textFor = (selector) => document.querySelector(selector)?.textContent?.trim() ?? ""
    const status = textFor("[data-waiting-room-footer-status]")
    const kernel = textFor("[data-waiting-room-footer-kernel]")
    const banner = textFor("[data-waiting-room-status]")
    return status === "ready"
      && Boolean(kernel)
      && !/no kernel connected|loading/i.test(kernel)
      && /Kernel waiting room ready/i.test(banner)
  }, timeoutMs, "product waiting room did not reach connected kernel state")
}

async function waitForWaitingRoomSessionRow(page, sessionId, timeoutMs) {
  await page.locator(`[data-waiting-session-id="${cssAttributeValue(sessionId)}"]`).first().waitFor({ timeout: timeoutMs })
}

async function waitForWaitingRoomSessionRowEnabled(page, sessionId, timeoutMs) {
  await waitForWebCondition(page, (targetSessionId) => {
    const row = document.querySelector(`[data-waiting-session-id="${targetSessionId}"]`)
    return Boolean(row)
      && !row.hasAttribute("disabled")
      && row.getAttribute("aria-disabled") !== "true"
      && !row.classList.contains("disabled")
  }, timeoutMs, `waiting-room session row ${sessionId} did not become enabled`, sessionId)
}

async function openSessionFromWaitingRoom(page, sessionId) {
  const joinRow = page.locator("[data-waiting-row-key='join']").first()
  await joinRow.click()
  const pickerRow = page.locator(`[data-session-picker-session-id="${cssAttributeValue(sessionId)}"]`).first()
  await pickerRow.waitFor({ timeout: 30_000 })
  await pickerRow.evaluate((element) => {
    if (element instanceof HTMLElement) element.click()
  })
}

async function clearSessionPickerOverlay(page) {
  const overlay = page.locator("[data-session-picker-close]").first()
  if (!(await overlay.isVisible().catch(() => false))) return
  await page.mouse.click(40, 40)
  await sleep(500)
  if (!(await overlay.isVisible().catch(() => false))) return
  await page.reload({ waitUntil: "domcontentloaded" })
  await page.locator("[data-freeform-pane-grid], .freeform-workspace").first().waitFor({ timeout: 90_000 })
}

async function expandWebTranscript(page, options) {
  const deadline = Date.now() + Math.min(30_000, Math.max(5_000, options.timeoutMs))
  while (Date.now() < deadline) {
    const clicked = await page.evaluate(() => {
      let count = 0
      for (const button of document.querySelectorAll('[data-freeform-turn-toggle][aria-expanded="false"]')) {
        if (button instanceof HTMLElement) {
          button.click()
          count += 1
        }
      }
      for (const button of document.querySelectorAll('.freeform-blob-header[aria-expanded="false"]')) {
        if (button instanceof HTMLElement) {
          button.click()
          count += 1
        }
      }
      return count
    }).catch(() => 0)
    if (!clicked) break
    await sleep(350)
  }
  await sleep(1_000)
}

async function expandTuiTranscriptBlobs(socketPath, options) {
  const deadline = Date.now() + Math.min(60_000, Math.max(5_000, options.timeoutMs))
  const attempts = new Map()
  while (Date.now() < deadline) {
    const snapshot = await automationRequest(socketPath, { action: "snapshot" }).catch(() => null)
    const entries = tuiSnapshotEntries(snapshot)
    const turnTargets = entries.filter(({ entry }) => {
      const turnId = Number(entry?.turnId)
      const entryId = Number(entry?.id)
      return Number.isInteger(turnId)
        && Number.isInteger(entryId)
        && entry?.role === "turn_toggle"
        && entry?.toggleMode === "expand"
        && (attempts.get(`turn:${entry.agentId ?? "primary"}:${turnId}:${entryId}`) ?? 0) < 3
    }).slice(0, 16)
    if (turnTargets.length > 0) {
      for (const target of turnTargets) {
        const turnId = Number(target.entry.turnId)
        const entryId = Number(target.entry.id)
        const key = `turn:${target.agentId ?? "primary"}:${turnId}:${entryId}`
        attempts.set(key, (attempts.get(key) ?? 0) + 1)
        await automationRequest(socketPath, {
          action: "toggle_turn",
          agentId: target.agentId,
          turnId,
          entryId,
        }).catch(() => null)
      }
      await sleep(750)
      continue
    }

    const blobTargets = entries.filter(({ entry }) => {
      const entryId = Number(entry?.id)
      const key = `blob:${entry.agentId ?? "primary"}:${String(entry?.historyBlobId ?? entryId)}`
      return Number.isInteger(entryId)
        && entry?.blobCollapsible === true
        && entry?.blobCollapsed !== false
        && (attempts.get(key) ?? 0) < 4
    }).slice(0, 24)
    if (blobTargets.length === 0) break
    for (const target of blobTargets) {
      const entryId = Number(target.entry.id)
      const key = `blob:${target.agentId ?? "primary"}:${String(target.entry.historyBlobId ?? entryId)}`
      attempts.set(key, (attempts.get(key) ?? 0) + 1)
      await automationRequest(socketPath, {
        action: "toggle_blob",
        agentId: target.agentId,
        entryId,
        collapsed: false,
      }).catch(() => null)
    }
    await sleep(750)
  }
  await sleep(1_000)
}

function tuiSnapshotEntries(snapshot) {
  if (!snapshot) return []
  const transcriptEntries = (snapshot.transcript?.entries ?? [])
    .filter(Boolean)
    .map((entry) => ({ entry, agentId: null }))
  const paneEntries = Object.entries(snapshot.agentPanes ?? {}).flatMap(([agentId, entries]) =>
    (entries ?? []).filter(Boolean).map((entry) => ({ entry, agentId })),
  )
  return [...transcriptEntries, ...paneEntries]
}

async function waitForWebCondition(page, predicate, timeoutMs, message, arg = undefined) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      if (await page.evaluate(predicate, arg)) return
    } catch (error) {
      lastError = error
    }
    await sleep(250)
  }
  throw new Error(`${message}${lastError ? `: ${lastError.message}` : ""}`)
}

function cssAttributeValue(value) {
  return String(value).replace(/\\/g, "\\\\").replace(/"/g, '\\"')
}

function startWebMonitor({ page, provider, marker, finalMarker, promptMarker, providerRoot, options }) {
  const samples = []
  let stopped = false
  const loop = (async () => {
    while (!stopped) {
      samples.push(await webSample(page, provider, marker, finalMarker, promptMarker).catch((error) => ({
        at: new Date().toISOString(),
        error: String(error?.message ?? error),
      })))
      await sleep(options.pollMs)
    }
  })()
  return {
    async stop() {
      stopped = true
      await loop.catch(() => {})
      samples.push(await webSample(page, provider, marker, finalMarker, promptMarker).catch((error) => ({ error: String(error?.message ?? error) })))
      await captureWebScreenshot(page, path.join(providerRoot, "web-final.png"))
      return summarizeSamples("web", samples, finalMarker)
    },
  }
}

async function captureWebScreenshot(page, file) {
  await page.screenshot({
    path: file,
    fullPage: false,
    timeout: 10_000,
  }).catch((error) => {
    void writeFile(`${file}.error.txt`, String(error?.stack ?? error), "utf8").catch(() => {})
  })
}

async function webSample(page, provider, marker, finalMarker, promptMarker) {
  return await page.evaluate(({ provider, marker, promptMarker, requiredAssistantMarkers, requiredToolMarkers, finalMarker }) => {
    const output = document.querySelector("[data-terminal-output]") ?? document.body
    const text = output.textContent ?? ""
    const scrollElement = output instanceof HTMLElement ? output : document.scrollingElement
    const bottomDistance = scrollElement
      ? Math.max(0, scrollElement.scrollHeight - scrollElement.clientHeight - scrollElement.scrollTop)
      : null
    const badges = [...document.querySelectorAll(".freeform-status-badge")].map((element) => element.textContent?.trim() ?? "")
    const turnButtons = [...document.querySelectorAll("[data-freeform-turn-toggle]")].map((element) => element.getAttribute("aria-expanded"))
    const blobButtons = [...document.querySelectorAll(".freeform-blob-header")].map((element) => element.getAttribute("aria-expanded"))
    return {
      at: new Date().toISOString(),
      surface: "web",
      status: badges.includes("WORKING") ? "WORKING" : badges.includes("IDLE") ? "IDLE" : "UNKNOWN",
      text,
      assistantMarkers: requiredAssistantMarkers.filter((entry) => text.includes(entry)),
      toolMarkers: requiredToolMarkers.filter((entry) => text.includes(entry)),
      finalSeen: text.includes(finalMarker),
      promptOccurrences: text.split(promptMarker).length - 1,
      bottomDistance,
      turnExpandedCount: turnButtons.filter((value) => value === "true").length,
      turnCollapsedCount: turnButtons.filter((value) => value === "false").length,
      blobExpandedCount: blobButtons.filter((value) => value === "true").length,
      blobCollapsedCount: blobButtons.filter((value) => value === "false").length,
      provider,
    }
  }, { provider, marker, promptMarker, requiredAssistantMarkers, requiredToolMarkers, finalMarker })
}

function summarizeSamples(surface, samples, finalMarker) {
  const valid = samples.filter((sample) => !sample.error)
  const text = valid.map((sample) => sample.text ?? "").join("\n")
  const firstFinalSampleIndex = valid.findIndex((sample) => sample.finalSeen)
  const preFinalSamples = firstFinalSampleIndex >= 0 ? valid.slice(0, firstFinalSampleIndex) : valid
  const finalAndLaterSamples = firstFinalSampleIndex >= 0 ? valid.slice(firstFinalSampleIndex) : []
  const countMax = (entries, key) => Math.max(0, ...entries.map((sample) => Number(sample[key] ?? 0)).filter(Number.isFinite))
  return {
    surface,
    sampleCount: samples.length,
    errorCount: samples.length - valid.length,
    samples,
    assistantMarkersSeen: requiredAssistantMarkers.filter((marker) => text.includes(marker)),
    toolMarkersSeen: requiredToolMarkers.filter((marker) => text.includes(marker)),
    finalSeen: text.includes(finalMarker),
    statuses: valid.map((sample) => sample.status).filter(Boolean),
    maxBottomDistance: Math.max(0, ...valid.map((sample) => Number(sample.bottomDistance ?? 0)).filter(Number.isFinite)),
    promptOccurrenceMax: Math.max(0, ...valid.map((sample) => Number(sample.promptOccurrences ?? 0)).filter(Number.isFinite)),
    firstFinalSampleIndex,
    preFinalSampleCount: preFinalSamples.length,
    preFinalMaxAssistantMarkers: Math.max(0, ...preFinalSamples.map((sample) => sample.assistantMarkers?.length ?? 0)),
    preFinalMaxToolMarkers: Math.max(0, ...preFinalSamples.map((sample) => sample.toolMarkers?.length ?? 0)),
    preFinalStatuses: preFinalSamples.map((sample) => sample.status).filter(Boolean),
    preFinalMaxTurnCollapsedCount: countMax(preFinalSamples, "turnCollapsedCount"),
    preFinalMaxTurnExpandedCount: countMax(preFinalSamples, "turnExpandedCount"),
    finalMaxTurnCollapsedCount: countMax(finalAndLaterSamples, "turnCollapsedCount"),
    finalMaxTurnExpandedCount: countMax(finalAndLaterSamples, "turnExpandedCount"),
    finalMaxBlobCollapsedCount: countMax(finalAndLaterSamples, "blobCollapsedCount"),
  }
}

async function snapshotProviderTranscript({ provider, providerSessionId, providerRoot, finalMarker, promptMarker }) {
  if (!providerSessionId) {
    return { surface: "provider", found: false, reason: "provider session id unavailable" }
  }
  if (provider === "opencode") {
    const sqliteSnapshot = await snapshotOpenCodeSqliteTranscript({ providerSessionId, providerRoot, finalMarker, promptMarker })
    if (sqliteSnapshot.found) return sqliteSnapshot
  }
  const path = await findProviderTranscriptPath(provider, providerSessionId)
  if (!path) {
    return {
      surface: "provider",
      found: false,
      providerSessionId,
      reason: `no ${provider} transcript path matched provider session ${providerSessionId}`,
    }
  }
  const text = await readFile(path, "utf8")
  const artifactPath = pathJoin(providerRoot, `provider-transcript${pathExt(path)}`)
  await writeFile(artifactPath, text, "utf8")
  return {
    surface: "provider",
    found: true,
    providerSessionId,
    path,
    artifactPath,
    byteLength: Buffer.byteLength(text),
    assistantMarkersSeen: requiredAssistantMarkers.filter((marker) => text.includes(marker)),
    toolMarkersSeen: requiredToolMarkers.filter((marker) => text.includes(marker)),
    finalSeen: text.includes(finalMarker),
    promptOccurrences: countOccurrences(text, promptMarker),
  }
}

async function snapshotOpenCodeSqliteTranscript({ providerSessionId, providerRoot, finalMarker, promptMarker }) {
  for (const root of providerTranscriptRoots("opencode")) {
    const dbPath = path.join(root, "opencode.db")
    if (!(await stat(dbPath).catch(() => null))) continue
    const sessionId = sqliteString(providerSessionId)
    const query = `
      select 'message' as kind, id, session_id, null as message_id, time_created, time_updated, data
        from message
       where session_id = '${sessionId}'
      union all
      select 'part' as kind, id, session_id, message_id, time_created, time_updated, data
        from part
       where session_id = '${sessionId}'
       order by time_created, id
    `
    const capture = await captureCommand("sqlite3", ["-json", dbPath, query], { maxBytes: 16 * 1024 * 1024 }).catch((error) => ({
      ok: false,
      stderr: String(error?.message ?? error),
    }))
    if (!capture.ok || !capture.stdout.trim()) continue
    const rows = parseJson(capture.stdout)
    if (!Array.isArray(rows) || rows.length === 0) continue
    const text = JSON.stringify(rows, null, 2)
    const artifactPath = pathJoin(providerRoot, "provider-transcript.sqlite.json")
    await writeFile(artifactPath, text, "utf8")
    return {
      surface: "provider",
      found: true,
      providerSessionId,
      path: dbPath,
      artifactPath,
      byteLength: Buffer.byteLength(text),
      rowCount: rows.length,
      assistantMarkersSeen: requiredAssistantMarkers.filter((marker) => text.includes(marker)),
      toolMarkersSeen: requiredToolMarkers.filter((marker) => text.includes(marker)),
      finalSeen: text.includes(finalMarker),
      promptOccurrences: countOccurrences(text, promptMarker),
    }
  }
  return {
    surface: "provider",
    found: false,
    providerSessionId,
    reason: `no OpenCode SQLite transcript rows matched provider session ${providerSessionId}`,
  }
}

function sqliteString(value) {
  return String(value).replace(/'/g, "''")
}

function captureCommand(command, args, { maxBytes = 1024 * 1024 } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"] })
    const stdout = []
    const stderr = []
    let stdoutBytes = 0
    let stderrBytes = 0
    child.stdout.on("data", (chunk) => {
      stdoutBytes += chunk.length
      if (stdoutBytes <= maxBytes) stdout.push(chunk)
      if (stdoutBytes > maxBytes) child.kill("SIGTERM")
    })
    child.stderr.on("data", (chunk) => {
      stderrBytes += chunk.length
      if (stderrBytes <= maxBytes) stderr.push(chunk)
    })
    child.on("error", reject)
    child.on("close", (code, signal) => {
      const result = {
        ok: code === 0,
        code,
        signal,
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
      }
      if (result.ok) resolve(result)
      else reject(new Error(`${command} exited with ${signal ?? code}: ${result.stderr}`))
    })
  })
}

async function findProviderTranscriptPath(provider, providerSessionId) {
  const roots = providerTranscriptRoots(provider)
  for (const root of roots) {
    const candidates = await providerTranscriptCandidates(provider, root)
    for (const candidate of candidates) {
      if (await providerTranscriptMatches(provider, candidate, providerSessionId)) {
        return candidate
      }
    }
  }
  return null
}

function providerTranscriptRoots(provider) {
  const home = os.homedir()
  if (provider === "codex") {
    return [process.env.CODEX_HOME || path.join(home, ".codex")]
  }
  if (provider === "claude") {
    return [process.env.CLAUDE_HOME || path.join(home, ".claude")]
  }
  if (provider === "opencode") {
    return dedupe([
      process.env.OPENCODE_DATA_HOME,
      process.env.XDG_DATA_HOME ? path.join(process.env.XDG_DATA_HOME, "opencode") : null,
      path.join(home, ".local", "share", "opencode"),
      path.join(home, ".config", "opencode"),
    ].filter(Boolean))
  }
  return []
}

async function providerTranscriptCandidates(provider, root) {
  const specs = {
    codex: [
      { root: path.join(root, "archived_sessions"), depth: 4, extensions: new Set([".jsonl"]) },
      { root: path.join(root, "sessions"), depth: 4, extensions: new Set([".jsonl"]) },
    ],
    claude: [
      { root: path.join(root, "projects"), depth: 3, extensions: new Set([".jsonl"]) },
    ],
    opencode: [
      { root, depth: 5, extensions: new Set([".json", ".jsonl"]) },
    ],
  }[provider] ?? []
  const files = []
  for (const spec of specs) {
    files.push(...await fileCandidates(spec.root, spec.depth, spec.extensions, provider === "opencode"))
  }
  await primeStatCache(files)
  return sortRecentFiles(dedupe(files)).slice(0, 1_000)
}

async function providerTranscriptMatches(provider, file, providerSessionId) {
  if (path.basename(file) === `${providerSessionId}.json` || path.basename(file) === `${providerSessionId}.jsonl`) {
    return true
  }
  if (path.basename(file).includes(providerSessionId)) {
    return true
  }
  const text = await readFile(file, "utf8").catch(() => "")
  if (!text) return false
  if (provider === "codex") {
    return jsonlValues(text).some((value) => value?.type === "session_meta"
      && stringField(value.payload, ["id", "session_id", "sessionId"]) === providerSessionId)
  }
  if (provider === "claude") {
    return jsonlValues(text).some((value) => stringField(value, ["sessionId", "session_id"]) === providerSessionId)
  }
  if (provider === "opencode") {
    if (path.extname(file) === ".jsonl") {
      return jsonlValues(text).some((value) => stringField(value, ["sessionID", "sessionId", "session_id", "id"]) === providerSessionId)
    }
    const value = parseJson(text)
    if (!value) return false
    return stringField(value, ["id", "sessionID", "sessionId", "session_id"]) === providerSessionId
  }
  return false
}

async function fileCandidates(root, depth, extensions, opencodeNamesOnly = false) {
  if (depth <= 0) return []
  const entries = await readdir(root, { withFileTypes: true }).catch(() => [])
  const files = []
  for (const entry of entries) {
    if (entry.name === "node_modules") continue
    const file = path.join(root, entry.name)
    if (entry.isDirectory()) {
      files.push(...await fileCandidates(file, depth - 1, extensions, opencodeNamesOnly))
    } else if (entry.isFile() && extensions.has(path.extname(entry.name))) {
      const lower = file.toLowerCase()
      if (!opencodeNamesOnly || lower.includes("session") || lower.includes("conversation") || lower.includes("message") || lower.endsWith(".jsonl")) {
        files.push(file)
      }
    }
  }
  return files
}

function sortRecentFiles(files) {
  return files.sort((left, right) => fileModifiedMs(right) - fileModifiedMs(left) || left.localeCompare(right))
}

function fileModifiedMs(file) {
  return statSyncCache.get(file) ?? 0
}

const statSyncCache = new Map()

async function primeStatCache(files) {
  await Promise.all(files.map(async (file) => {
    const metadata = await stat(file).catch(() => null)
    statSyncCache.set(file, metadata?.mtimeMs ?? 0)
  }))
}

function jsonlValues(text) {
  return String(text)
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      try {
        return JSON.parse(line)
      } catch {
        return null
      }
    })
    .filter(Boolean)
}

function parseJson(text) {
  try {
    return JSON.parse(text)
  } catch {
    return null
  }
}

function stringField(value, keys) {
  if (!value || typeof value !== "object") return null
  for (const key of keys) {
    const field = value[key]
    if (typeof field === "string" && field.length > 0) return field
  }
  return null
}

function pathJoin(...parts) {
  return path.join(...parts)
}

function pathExt(file) {
  const extension = path.extname(file)
  return extension || ".txt"
}

function assertProviderTranscript(result, transcript, label) {
  result.assertions.push(assertion(`${label} file found`, Boolean(transcript?.found), transcript?.reason ?? transcript?.path))
  if (!transcript?.found) return
  result.assertions.push(assertion(`${label} saw all assistant markers`, transcript.assistantMarkersSeen.length === 20, transcript.assistantMarkersSeen))
  result.assertions.push(assertion(`${label} saw all tool markers`, transcript.toolMarkersSeen.length === 20, transcript.toolMarkersSeen))
  result.assertions.push(assertion(`${label} saw final summary marker`, transcript.finalSeen, transcript.finalSeen))
  result.assertions.push(assertion(`${label} saw external prompt marker`, transcript.promptOccurrences >= 1, transcript.promptOccurrences))
}

function dedupe(values) {
  return [...new Set(values)]
}

function assertSurface(result, surfaceResult, label) {
  if (!surfaceResult) {
    result.assertions.push(fail(`${label} monitor ran`, "missing monitor result"))
    return
  }
  result.assertions.push(assertion(`${label} saw all assistant markers`, surfaceResult.assistantMarkersSeen.length === 20, surfaceResult.assistantMarkersSeen))
  result.assertions.push(assertion(`${label} saw all tool markers`, surfaceResult.toolMarkersSeen.length === 20, surfaceResult.toolMarkersSeen))
  result.assertions.push(assertion(`${label} saw final summary marker`, surfaceResult.finalSeen, surfaceResult.finalSeen))
  result.assertions.push(assertion(`${label} rendered external prompt marker exactly once`, surfaceResult.promptOccurrenceMax === 1, surfaceResult.promptOccurrenceMax))
  if (label.includes("web")) {
    result.assertions.push(assertion(`${label} stayed near bottom while tailing`, surfaceResult.maxBottomDistance < 260, surfaceResult.maxBottomDistance))
  }
}

function assertLiveObservation(result, surfaceResult, label) {
  if (!surfaceResult) return
  result.assertions.push(assertion(`${label} sampled turn before final summary`, surfaceResult.preFinalSampleCount > 0, {
    preFinalSampleCount: surfaceResult.preFinalSampleCount,
    firstFinalSampleIndex: surfaceResult.firstFinalSampleIndex,
  }))
  const preFinalStatuses = surfaceResult.preFinalStatuses.map(normalizeLifecycleStatus)
  result.assertions.push(assertion(`${label} observed active pre-final lifecycle`, preFinalStatuses.includes("WORKING"), preFinalStatuses))
  const sawPreFinalContent = surfaceResult.preFinalMaxAssistantMarkers > 0 || surfaceResult.preFinalMaxToolMarkers > 0
  result.assertions.push(assertion(`${label} observed live pre-final content`, sawPreFinalContent, {
    assistantMarkers: surfaceResult.preFinalMaxAssistantMarkers,
    toolMarkers: surfaceResult.preFinalMaxToolMarkers,
  }))
}

function assertBadgeLifecycle(result, surfaceResult, label) {
  if (!surfaceResult) return
  const statuses = surfaceResult.statuses.map(normalizeLifecycleStatus)
  result.assertions.push(assertion(`${label} observed WORKING`, statuses.includes("WORKING"), statuses))
  result.assertions.push(assertion(`${label} ended IDLE or unknown-idle-compatible`, statuses.at(-1) === "IDLE" || statuses.at(-1) === "IDLE/DONE" || statuses.at(-1) === "DONE", statuses.at(-1)))
  const firstWorking = statuses.indexOf("WORKING")
  const finalIndex = surfaceResult.samples.findIndex((sample) => sample.finalSeen)
  const prematureIdle = firstWorking >= 0 && finalIndex > firstWorking
    ? statuses.slice(firstWorking, finalIndex).some((status) => status === "IDLE" || status === "DONE")
    : false
  result.assertions.push(assertion(`${label} did not go idle before final summary`, !prematureIdle, statuses))
}

function assertWebTurnCollapse(result, webResult) {
  result.assertions.push(assertion("web collapsed completed turn after final summary", webResult.finalMaxTurnCollapsedCount > 0, {
    finalCollapsed: webResult.finalMaxTurnCollapsedCount,
    finalExpanded: webResult.finalMaxTurnExpandedCount,
    finalBlobCollapsed: webResult.finalMaxBlobCollapsedCount,
  }))
}

function normalizeLifecycleStatus(status) {
  const normalized = String(status ?? "").trim().toUpperCase()
  if (["WORKING", "RUNNING", "THINKING", "STREAMING", "BUSY"].includes(normalized)) return "WORKING"
  return normalized
}

function providerLimitations(provider, monitorResults, context = {}) {
  const surfaceTexts = new Map([
    ["provider_transcript", monitorResults.providerTranscript?.found ? readArtifactTextSync(monitorResults.providerTranscript) : ""],
    ["kernel", monitorResults.kernel?.samples?.map((sample) => sample.text ?? "").join("\n") ?? ""],
    ["web", monitorResults.web?.samples?.map((sample) => sample.text ?? "").join("\n") ?? ""],
    ["tui", monitorResults.tui?.samples?.map((sample) => sample.text ?? "").join("\n") ?? ""],
  ])
  const combinedText = [...surfaceTexts.values()].join("\n").toLowerCase()
  const surfacesWithText = (predicate) => [...surfaceTexts.entries()]
    .filter(([, text]) => text && predicate(text.toLowerCase()))
    .map(([surface]) => surface)
  const metadataReport = [
    metadataAvailability({
      provider,
      context,
      metadata: "status/running_state",
      observed: Boolean(monitorResults.kernel?.statuses?.length || monitorResults.web?.statuses?.length || monitorResults.tui?.statuses?.length),
      surfaces: ["kernel", monitorResults.web ? "web" : null, monitorResults.tui ? "tui" : null].filter(Boolean),
      observedNote: "Arroba observed external turn lifecycle from WORKING to final IDLE/DONE.",
      missingNote: "No lifecycle statuses were sampled during the drill.",
      missingClassification: "arroba_bug",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "assistant_text",
      observed: (monitorResults.kernel?.assistantMarkersSeen?.length ?? 0) === 20,
      surfaces: surfacesWithText((text) => requiredAssistantMarkers.every((marker) => text.includes(marker.toLowerCase()))),
      observedNote: "All assistant progress markers were visible in imported external history.",
      missingNote: "Assistant text did not fully appear in imported external history.",
      missingClassification: "arroba_bug",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "tool_calls",
      observed: (monitorResults.kernel?.toolMarkersSeen?.length ?? 0) === 20,
      surfaces: surfacesWithText((text) => requiredToolMarkers.every((marker) => text.includes(marker.toLowerCase()))),
      observedNote: "All marked provider tool calls were visible in imported external history.",
      missingNote: "Tool-call markers did not fully appear in imported external history.",
      missingClassification: "arroba_bug",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "tool_results",
      observed: requiredToolMarkers.some((marker) => combinedText.includes(marker.toLowerCase())),
      surfaces: surfacesWithText((text) => requiredToolMarkers.some((marker) => text.includes(marker.toLowerCase()))),
      observedNote: "Provider tool result/output text was visible at least through marked tool-call output.",
      missingNote: "Tool result/output text was not observed in imported external history.",
      missingClassification: "provider_persistence_or_adapter_limitation",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "reasoning/thinking_summaries",
      observed: /\b(reasoning|thinking|thought|summary unavailable|visible summary)\b/i.test(combinedText),
      surfaces: surfacesWithText((text) => /\b(reasoning|thinking|thought|summary unavailable|visible summary)\b/i.test(text)),
      observedNote: "Reasoning/thinking metadata or an explicit unavailable-summary entry was visible.",
      missingNote: "Reasoning/thinking summaries were not observed in imported external history.",
      missingClassification: "provider_persistence_limitation",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "timestamps",
      observed: Boolean(monitorResults.kernel?.samples?.some((sample) => sample.at) || monitorResults.providerTranscript?.found),
      surfaces: ["provider_transcript", "kernel"].filter((surface) => surface !== "provider_transcript" || monitorResults.providerTranscript?.found),
      observedNote: "Observation timestamps and/or provider transcript timestamps were captured.",
      missingNote: "No timestamps were captured for provider or Arroba observations.",
      missingClassification: "drill_observation_limitation",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "token_usage",
      observed: /\b(token|usage|input_tokens|output_tokens|cached_input_tokens)\b/i.test(combinedText),
      surfaces: surfacesWithText((text) => /\b(token|usage|input_tokens|output_tokens|cached_input_tokens)\b/i.test(text)),
      observedNote: "Token usage metadata was visible in provider/imported history.",
      missingNote: "Token usage metadata was not observed in imported external history.",
      missingClassification: "provider_persistence_limitation",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "model_identity",
      observed: Boolean(context.model) || /\b(model|gpt|sonnet|kimi|claude)\b/i.test(combinedText),
      surfaces: context.model ? ["drill_config"] : surfacesWithText((text) => /\b(model|gpt|sonnet|kimi|claude)\b/i.test(text)),
      observedNote: `Model identity was available to the drill as ${context.model ?? "provider transcript metadata"}.`,
      missingNote: "Model identity was not observed.",
      missingClassification: "drill_observation_limitation",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "final_completion_or_error_state",
      observed: Boolean(monitorResults.kernel?.finalSeen || context.providerExit),
      surfaces: ["provider_process", "kernel"].filter((surface) => surface !== "kernel" || monitorResults.kernel?.finalSeen),
      observedNote: `Provider exit and final marker were captured${context.providerExit ? ` with exit ${JSON.stringify(context.providerExit)}` : ""}.`,
      missingNote: "Final completion/error state was not captured.",
      missingClassification: "arroba_bug",
    }),
  ]
  if (!monitorResults.web) metadataReport.push({ provider, surface: "web", status: "skipped", classification: "drill_observation_limitation" })
  if (!monitorResults.tui) metadataReport.push({ provider, surface: "tui", status: "skipped", classification: "drill_observation_limitation" })
  return metadataReport
}

function metadataAvailability({ provider, context, metadata, observed, surfaces, observedNote, missingNote, missingClassification }) {
  return {
    provider,
    providerSessionId: context.providerSessionId ?? null,
    externalSessionId: context.externalSessionId ?? null,
    arrobaSessionId: context.arrobaSessionId ?? null,
    agentId: context.agentId ?? null,
    metadata,
    status: observed ? "observed" : "not_observed",
    classification: observed ? "available_to_arroba" : missingClassification,
    surfaces: dedupe((surfaces ?? []).filter(Boolean)),
    note: observed ? observedNote : missingNote,
  }
}

function readArtifactTextSync(transcript) {
  const artifactPath = transcript.artifactPath ?? transcript.path
  if (!artifactPath) return ""
  try {
    return readFileSync(artifactPath, "utf8")
  } catch {
    return [
      transcript.path ?? "",
      transcript.artifactPath ?? "",
      ...(transcript.assistantMarkersSeen ?? []),
      ...(transcript.toolMarkersSeen ?? []),
      transcript.finalSeen ? "final" : "",
    ].join("\n")
  }
}

async function waitForAutomation(socketPath, child) {
  const deadline = Date.now() + 90_000
  let lastError = null
  while (Date.now() < deadline) {
    if (child.exitCode != null) throw new Error(`TUI exited before automation socket became ready: ${child.exitCode}`)
    try {
      await automationRequest(socketPath, { action: "ping" }, 5_000)
      return
    } catch (error) {
      lastError = error
      await sleep(250)
    }
  }
  throw new Error(`automation socket did not become ready: ${lastError?.message ?? lastError}`)
}

async function automationRequest(socketPath, request, timeoutMs = 20_000) {
  return await new Promise((resolve, reject) => {
    const socket = net.createConnection(socketPath)
    let buffer = ""
    socket.setTimeout(timeoutMs)
    socket.once("error", reject)
    socket.once("timeout", () => reject(new Error(`automation request timed out: ${JSON.stringify(request)}`)))
    socket.on("data", (chunk) => {
      buffer += chunk.toString("utf8")
      const index = buffer.indexOf("\n")
      if (index < 0) return
      const line = buffer.slice(0, index)
      socket.end()
      const response = JSON.parse(line)
      if (!response.ok) reject(new Error(response.error ?? "automation request failed"))
      else resolve(response.data)
    })
    socket.once("connect", () => {
      socket.write(`${JSON.stringify({ id: Date.now(), ...request })}\n`)
    })
  })
}

function pipeChildLogs(child, stdoutPath, stderrPath) {
  let stdout = ""
  let stderr = ""
  child.stdout?.on("data", (chunk) => {
    stdout += chunk.toString("utf8")
    if (stdout.length > 250_000) stdout = stdout.slice(-250_000)
    void writeFile(stdoutPath, stdout, "utf8").catch(() => {})
  })
  child.stderr?.on("data", (chunk) => {
    stderr += chunk.toString("utf8")
    if (stderr.length > 250_000) stderr = stderr.slice(-250_000)
    void writeFile(stderrPath, stderr, "utf8").catch(() => {})
  })
}

async function closeWithTimeout(target, label) {
  if (!target?.close) return
  let timedOut = false
  await Promise.race([
    target.close(),
    sleep(3_000).then(() => {
      timedOut = true
      console.warn(`${label} close timed out`)
    }),
  ]).catch(() => {})
  if (timedOut && typeof target.process === "function") {
    target.process()?.kill("SIGKILL")
  }
}

function stopChild(child) {
  if (!child || child.exitCode != null) return
  child.kill("SIGTERM")
  setTimeout(() => {
    if (child.exitCode == null) child.kill("SIGKILL")
  }, 2_000).unref()
}

function countOccurrences(text, needle) {
  if (!needle) return 0
  return String(text).split(needle).length - 1
}

function pass(name) {
  return { name, passed: true }
}

function fail(name, details = null) {
  return { name, passed: false, details }
}

function assertion(name, passed, details = null) {
  return { name, passed: Boolean(passed), details }
}

async function writeJson(file, value) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify(value, null, 2)}\n`, "utf8")
}

async function writeFinalReport(root, summary) {
  const report = [
    "# External Provider Live Parity Drill Report",
    "",
    "## Overall Result",
    "",
    `- Result: ${summary.ok ? "PASS" : "FAIL"}`,
    `- Created: ${summary.createdAt}`,
    `- Artifact root: ${summary.artifactRoot}`,
    `- Kernel URL: ${summary.kernelUrl}`,
    `- Web URL: ${summary.webUrl}`,
    `- Workspace: ${summary.workspace}`,
    "",
    "## Provider Matrix",
    "",
    "| Provider | Model | Provider session | Arroba session | Agent | Result | Failed assertions |",
    "| --- | --- | --- | --- | --- | --- | --- |",
    ...summary.results.map((result) => {
      const failed = (result.assertions ?? []).filter((assertion) => !assertion.passed)
      return [
        result.provider,
        result.model,
        result.providerSessionId ?? result.externalSessionId ?? "",
        result.arrobaSessionId ?? "",
        result.agentId ?? "",
        result.ok ? "PASS" : "FAIL",
        failed.map((assertion) => assertion.name).join("; "),
      ].map(markdownCell).join(" | ").replace(/^/, "| ").replace(/$/, " |")
    }),
    "",
    "## Web Terminal Evidence",
    "",
    ...surfaceEvidence(summary.results, "web"),
    "",
    "## TUI Evidence",
    "",
    ...surfaceEvidence(summary.results, "tui"),
    "",
    "## Artifact Paths",
    "",
    ...summary.results.flatMap((result) => [
      `- ${result.provider}: ${result.artifactDir}`,
      `  - Prompt: ${path.join(result.artifactDir, "prompt.txt")}`,
      `  - Provider stdout: ${path.join(result.artifactDir, "provider.stdout.log")}`,
      `  - Provider stderr: ${path.join(result.artifactDir, "provider.stderr.log")}`,
      `  - Web final screenshot: ${path.join(result.artifactDir, "web-final.png")}`,
      `  - Provider transcript artifact: ${result.evidence?.providerTranscript?.artifactPath ?? "not captured"}`,
    ]),
    "",
    "## Provider Limitations And Clarifications",
    "",
    "This section is intentionally last. It distinguishes Arroba bugs from provider-native metadata limits and drill-observation limits.",
    "",
    "| Provider | Metadata | Status | Classification | Surfaces | Clarification | IDs |",
    "| --- | --- | --- | --- | --- | --- | --- |",
    ...summary.providerLimitations.map((entry) => [
      entry.provider ?? "",
      entry.metadata ?? entry.surface ?? "",
      entry.status ?? "",
      entry.classification ?? "",
      (entry.surfaces ?? [entry.surface]).filter(Boolean).join(", "),
      entry.note ?? "",
      [
        entry.providerSessionId ? `provider=${entry.providerSessionId}` : null,
        entry.externalSessionId ? `external=${entry.externalSessionId}` : null,
        entry.arrobaSessionId ? `arroba=${entry.arrobaSessionId}` : null,
        entry.agentId ? `agent=${entry.agentId}` : null,
      ].filter(Boolean).join("; "),
    ].map(markdownCell).join(" | ").replace(/^/, "| ").replace(/$/, " |")),
    "",
  ].join("\n")
  await writeFile(path.join(root, "final-report.md"), report, "utf8")
}

function surfaceEvidence(results, surface) {
  const lines = []
  for (const result of results) {
    const monitor = result.evidence?.monitors?.find((entry) => entry.surface === surface)
    if (!monitor) {
      lines.push(`- ${result.provider}: ${surface} monitor was skipped or unavailable.`)
      continue
    }
    const statuses = monitor.statuses ?? []
    lines.push([
      `- ${result.provider}:`,
      `result=${result.ok ? "PASS" : "FAIL"}`,
      `samples=${monitor.sampleCount}`,
      `assistant=${monitor.assistantMarkersSeen?.length ?? 0}/20`,
      `tools=${monitor.toolMarkersSeen?.length ?? 0}/20`,
      `final=${monitor.finalSeen ? "yes" : "no"}`,
      `first_status=${statuses[0] ?? "unknown"}`,
      `last_status=${statuses.at(-1) ?? "unknown"}`,
      `prompt_occurrence_max=${monitor.promptOccurrenceMax ?? "unknown"}`,
      surface === "web" ? `max_bottom_distance=${monitor.maxBottomDistance ?? "unknown"}` : null,
      `pre_final_samples=${monitor.preFinalSampleCount ?? 0}`,
    ].filter(Boolean).join(" "))
  }
  return lines
}

function markdownCell(value) {
  return String(value ?? "")
    .replace(/\|/g, "\\|")
    .replace(/\r?\n/g, "<br>")
}

main().catch((error) => {
  console.error(error?.stack ?? String(error))
  process.exit(1)
})
