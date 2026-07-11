#!/usr/bin/env node
import { spawn } from "node:child_process"
import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises"
import path from "node:path"
import { setTimeout as sleep } from "node:timers/promises"

import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"
import {
  fail,
  finalMarkerFor,
  finalMarkerPrefix,
  pass,
  unwrap,
  writeJson,
} from "./lib/live-external-provider-live-parity-common.mjs"
import {
  assertBadgeLifecycle,
  assertLiveObservation,
  assertProviderTranscript,
  assertSurface,
  assertWebTurnCollapse,
  providerLimitations,
  snapshotProviderTranscript,
  writeFinalReport,
} from "./lib/live-external-provider-live-parity-evidence.mjs"
import {
  expandTuiTranscriptBlobs,
  expandWebTranscript,
  startKernelMonitor,
  startTuiMonitor,
  startTuiObserver,
  startWebMonitor,
  startWebObserver,
  tuiSample,
  waitForKernelFinalIdle,
  waitForSurfaceFinalIdle,
  webSample,
} from "./lib/live-external-provider-live-parity-observers.mjs"
import { closeWithTimeout, stopChild } from "./lib/live-external-provider-live-parity-process.mjs"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  importExternalProviderSessionRequest,
  listExternalProviderSessionsRequest,
} from "../dist/ipc-requests.js"

const scriptDir = path.dirname(new URL(import.meta.url).pathname)
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const defaultKernelUrl = process.env.ARROBA_EXTERNAL_PARITY_KERNEL_URL ?? "ws://127.0.0.1:44120/kernel"
const defaultWebUrl = process.env.ARROBA_EXTERNAL_PARITY_WEB_URL ?? "http://127.0.0.1:4321"
const defaultProviders = ["codex", "claude", "opencode"]
const defaultModels = {
  codex: process.env.ARROBA_EXTERNAL_PARITY_CODEX_MODEL ?? "gpt-5.5",
  claude: process.env.ARROBA_EXTERNAL_PARITY_CLAUDE_MODEL ?? "sonnet",
  opencode: process.env.ARROBA_EXTERNAL_PARITY_OPENCODE_MODEL ?? "opencode/kimi-k2.6",
}
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
    skipKernelHistory: false,
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
    } else if (arg === "--skip-kernel-history") {
      options.skipKernelHistory = true
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
  --skip-kernel-history  Validate provider-native transcript parity without attached-history observation
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
    "9. Every marked tool command, including read/list/count/stat/test/delete commands, must begin by printing its own TOOL_STEP_NN marker with printf before doing anything else.",
    "10. Do not rely on a prior file's contents to make a later tool marker visible. The current tool call itself must print the current marker.",
    "11. Use only workspace-root temporary drill files whose paths begin with the scratch file prefix.",
    "12. Create, append, read, list, inspect metadata, and delete small text files with that scratch file prefix.",
    "13. Delete the temporary scratch files before finishing, but do not delete the observer gate go file.",
    `14. End with the exact final summary marker formed by joining prefix ${finalMarkerPrefix}, an underscore, and the drill marker.`,
    "15. The final summary marker must be the last assistant text and must be emitted only after the TOOL_STEP_20 tool call has completed.",
    "16. Do not repeat the user prompt marker in assistant progress messages, tool command text, tool output, or the final summary.",
    "17. Use only low-risk shell commands: printf, cat, ls, wc, stat, find, test, touch, rm, rmdir, and sleep.",
    "18. Do not use xattr, chmod, chown, install, Python, Node, Ruby, Perl, network commands, package managers, git, or any command likely to require interactive approval.",
    "19. Include a short `sleep 0.5` in each marked tool call so Arroba can observe the live turn before the final summary.",
  ].join("\n")
  return { text, promptMarker }
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
  console.log(JSON.stringify(drillConsoleSummary(summary), null, 2))
  process.exit(0)
}

function collectProviderLimitations(results) {
  return results.flatMap((result) => result.providerLimitations ?? [])
}

function drillConsoleSummary(summary) {
  return {
    ok: summary.ok,
    createdAt: summary.createdAt,
    artifactRoot: summary.artifactRoot,
    finalReport: path.join(summary.artifactRoot, "final-report.md"),
    results: summary.results.map((result) => ({
      provider: result.provider,
      ok: result.ok,
      providerSessionId: result.providerSessionId,
      externalSessionId: result.externalSessionId,
      arrobaSessionId: result.arrobaSessionId,
      agentId: result.agentId,
      artifactDir: result.artifactDir,
      failedAssertions: (result.assertions ?? [])
        .filter((assertion) => !assertion.passed)
        .map((assertion) => assertion.name),
    })),
  }
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
      providerProcess,
      providerRoot,
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
    if (!options.skipKernelHistory) {
      monitors.push(startKernelMonitor({ client, sessionId: result.arrobaSessionId, agentId: result.agentId, provider, marker, finalMarker, promptMarker: prompt.promptMarker, options }))
    }
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
    const providerExitIssue = await providerExitWithoutFinalMarker({
      provider,
      providerRoot,
      providerExit,
      finalMarker,
      result,
    })
    if (providerExitIssue) {
      result.providerLimitations.push(providerExitIssue)
      throw new Error(providerExitIssue.note)
    }
    if (!options.skipKernelHistory) {
      await waitForKernelFinalIdle({ client, sessionId: result.arrobaSessionId, agentId: result.agentId, provider, marker, finalMarker, promptMarker: prompt.promptMarker, options })
    }
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
    if (!options.skipKernelHistory) assertSurface(result, kernel, "kernel history")
    if (!options.skipWeb) assertSurface(result, web, "product web terminal")
    if (!options.skipTui) assertSurface(result, tui, "TUI")
    if (!options.skipKernelHistory) assertLiveObservation(result, kernel, "kernel", { requireContent: false })
    if (web) assertLiveObservation(result, web, "web")
    if (tui) assertLiveObservation(result, tui, "tui")
    if (!options.skipKernelHistory) assertBadgeLifecycle(result, kernel, "kernel")
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

async function waitForNewExternalSession({ client, provider, before, marker, providerProcess, providerRoot, timeoutMs, pollMs }) {
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
    if (providerProcess && (providerProcess.exitCode !== null || providerProcess.signalCode !== null)) {
      const stdout = await readLogTail(path.join(providerRoot, "provider.stdout.log"))
      const stderr = await readLogTail(path.join(providerRoot, "provider.stderr.log"))
      throw new Error(`provider ${provider} exited before a new external session appeared; code=${providerProcess.exitCode} signal=${providerProcess.signalCode ?? "none"} stdout=${stdout} stderr=${stderr}`)
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for new ${provider} external session; last=${JSON.stringify(last.slice(0, 5), null, 2)}`)
}

async function readLogTail(file) {
  const text = await readFile(file, "utf8").catch(() => "")
  return JSON.stringify(text.slice(-2000))
}

async function providerExitWithoutFinalMarker({ provider, providerRoot, providerExit, finalMarker, result }) {
  const stdout = await readFile(path.join(providerRoot, "provider.stdout.log"), "utf8").catch(() => "")
  const stderr = await readFile(path.join(providerRoot, "provider.stderr.log"), "utf8").catch(() => "")
  if (stdout.includes(finalMarker) || stderr.includes(finalMarker)) return null
  const combined = `${stdout}\n${stderr}`.trim()
  const tail = combined.slice(-2000)
  const providerLimitPattern = /\b(session limit|rate limit|quota|usage limit|limit reached|resets? at|resets?\s+\d|too many requests|insufficient (?:balance|credits?)|credits? error|billing|token refresh failed|unauthorized|401)\b/i
  const providerLimit = providerLimitPattern.test(combined)
  const abnormalExit = Boolean(providerExit?.error || providerExit?.signal || (providerExit?.code != null && providerExit.code !== 0))
  if (!providerLimit && !abnormalExit) return null
  const classification = providerLimit
    ? "provider_runtime_limitation"
    : "provider_execution_failure"
  const note = [
    `${provider} provider process exited before emitting the final marker.`,
    `exit=${JSON.stringify(providerExit)}`,
    tail ? `output_tail=${JSON.stringify(tail)}` : "output_tail=<empty>",
  ].join(" ")
  return {
    provider,
    providerSessionId: result.providerSessionId ?? null,
    externalSessionId: result.externalSessionId ?? null,
    arrobaSessionId: result.arrobaSessionId ?? null,
    agentId: result.agentId ?? null,
    metadata: "provider_execution",
    status: "not_observed",
    classification,
    surfaces: ["provider_process", "provider_stdout", "provider_stderr"],
    note,
  }
}

main().catch((error) => {
  console.error(error?.stack ?? String(error))
  process.exit(1)
})
