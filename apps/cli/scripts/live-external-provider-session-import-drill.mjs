import { spawn, execFile } from "node:child_process"
import { existsSync, readdirSync } from "node:fs"
import { access, mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  createSessionRequest,
  endSessionRequest,
  importExternalProviderAgentRequest,
  importExternalProviderSessionRequest,
  listExternalProviderSessionsRequest,
} from "../dist/ipc-requests.js"

const execFileAsync = promisify(execFile)
const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const kernelBinary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
const cloudRepoRoot = process.env.ARROBA_CLOUD_REPO
  ? path.resolve(process.env.ARROBA_CLOUD_REPO)
  : path.resolve(repoRoot, "..", "arroba-cloud")
const providers = ["codex", "opencode", "claude"]
const STEP_TIMEOUT_MS = 15_000
let browserPromise = null

function unwrap(response, variant) {
  if (!response || !(variant in response)) {
    throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  }
  return response[variant]
}

function makePort() {
  return 54000 + Math.floor(Math.random() * 2000)
}

function nowStamp() {
  return new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z")
}

async function waitForDaemon(kernelUrl, workspace) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      const session = unwrap(
        await client.send(createSessionRequest(workspace, workspace, "external-drill-probe")),
        "SessionCreated",
      ).session
      await client.send(endSessionRequest(session.id)).catch(() => {})
      await client.close()
      return
    } catch {
      await client.close().catch(() => {})
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }
  throw new Error("kernel did not become ready")
}

async function ensureKernelBinary() {
  try {
    await access(kernelBinary)
  } catch {
    await execFileAsync("cargo", [
      "build",
      "--manifest-path",
      path.join(repoRoot, "apps/kernel/Cargo.toml"),
      "--bin",
      "arroba-kernel",
    ], { cwd: repoRoot, stdio: "inherit" })
  }
}

async function seedProviderHomes(root, marker, workspace) {
  const codexHome = path.join(root, "provider-homes", "codex")
  const claudeHome = path.join(root, "provider-homes", "claude")
  const opencodeHome = path.join(root, "provider-homes", "opencode")
  await mkdir(path.join(codexHome, "sessions"), { recursive: true })
  await mkdir(path.join(claudeHome, "projects", "-repo"), { recursive: true })
  await mkdir(path.join(opencodeHome, "sessions"), { recursive: true })

  await writeFile(
    path.join(codexHome, "sessions", "codex-thread-drill.jsonl"),
    [
      JSON.stringify({
        timestamp: "2026-06-09T12:00:00.000Z",
        type: "session_meta",
        payload: { id: `codex-${marker}`, cwd: workspace, model_provider: "openai" },
      }),
      JSON.stringify({
        timestamp: "2026-06-09T12:00:01.000Z",
        type: "response_item",
        payload: {
          id: "codex-user-1",
          type: "message",
          role: "user",
          content: [{ type: "input_text", text: `Codex external drill prompt ${marker}.` }],
        },
      }),
      JSON.stringify({
        timestamp: "2026-06-09T12:00:02.000Z",
        type: "response_item",
        payload: {
          id: "codex-assistant-1",
          type: "message",
          role: "assistant",
          content: [{ type: "output_text", text: `Codex observed reply ${marker}.` }],
        },
      }),
    ].join("\n") + "\n",
  )

  await writeFile(
    path.join(claudeHome, "projects", "-repo", "claude-session-drill.jsonl"),
    [
      JSON.stringify({
        type: "user",
        uuid: "claude-user-1",
        sessionId: `claude-${marker}`,
        cwd: workspace,
        timestamp: "2026-06-09T12:00:01.000Z",
        message: { role: "user", content: [{ text: `Claude external drill prompt ${marker}.` }] },
      }),
      JSON.stringify({
        type: "assistant",
        uuid: "claude-assistant-1",
        sessionId: `claude-${marker}`,
        timestamp: "2026-06-09T12:00:02.000Z",
        message: { role: "assistant", content: [{ text: `Claude observed reply ${marker}.` }] },
      }),
    ].join("\n") + "\n",
  )

  await writeFile(
    path.join(opencodeHome, "sessions", "opencode-session-drill.json"),
    JSON.stringify({
      id: `opencode-${marker}`,
      title: `OpenCode external drill ${marker}`,
      cwd: workspace,
      updatedAt: "2026-06-09T12:00:03.000Z",
      messages: [
        {
          id: "opencode-user-1",
          role: "user",
          content: `OpenCode external drill prompt ${marker}.`,
          createdAt: "2026-06-09T12:00:01.000Z",
        },
        {
          id: "opencode-assistant-1",
          role: "assistant",
          content: `OpenCode observed reply ${marker}.`,
          createdAt: "2026-06-09T12:00:02.000Z",
        },
      ],
    }, null, 2),
  )

  return { CODEX_HOME: codexHome, CLAUDE_HOME: claudeHome, OPENCODE_DATA_HOME: opencodeHome }
}

async function appendProviderNativeTurn(root, provider, marker) {
  const providerSessionId = `${provider}-${marker}`
  if (provider === "codex") {
    const file = path.join(root, "provider-homes", "codex", "sessions", "codex-thread-drill.jsonl")
    await writeFile(
      file,
      JSON.stringify({
        timestamp: new Date().toISOString(),
        type: "response_item",
        payload: {
          id: "codex-assistant-2",
          type: "message",
          role: "assistant",
          content: [{ type: "output_text", text: `Codex native follow-up observed ${marker}.` }],
        },
      }) + "\n",
      { flag: "a" },
    )
    return { providerTurnId: "codex-assistant-2", text: `Codex native follow-up observed ${marker}.` }
  }
  if (provider === "claude") {
    const file = path.join(root, "provider-homes", "claude", "projects", "-repo", "claude-session-drill.jsonl")
    await writeFile(
      file,
      JSON.stringify({
        type: "assistant",
        uuid: "claude-assistant-2",
        sessionId: providerSessionId,
        timestamp: new Date().toISOString(),
        message: { role: "assistant", content: [{ text: `Claude native follow-up observed ${marker}.` }] },
      }) + "\n",
      { flag: "a" },
    )
    return { providerTurnId: "claude-assistant-2", text: `Claude native follow-up observed ${marker}.` }
  }
  const file = path.join(root, "provider-homes", "opencode", "sessions", "opencode-session-drill.json")
  const payload = JSON.parse(await readFile(file, "utf8"))
  payload.updatedAt = new Date().toISOString()
  payload.messages.push({
    id: "opencode-assistant-2",
    role: "assistant",
    content: `OpenCode native follow-up observed ${marker}.`,
    createdAt: new Date().toISOString(),
  })
  await writeFile(file, JSON.stringify(payload, null, 2))
  return { providerTurnId: "opencode-assistant-2", text: `OpenCode native follow-up observed ${marker}.` }
}

async function readHistoryEntries(historyRoot, sessionId) {
  const files = await readdir(historyRoot).catch(() => [])
  const historyFile = files.find((file) => file.startsWith(`${sessionId}-`) && file.endsWith(".jsonl"))
  if (!historyFile) return []
  const payload = await readFile(path.join(historyRoot, historyFile), "utf8")
  const entries = []
  for (const line of payload.split("\n")) {
    if (!line.trim()) continue
    entries.push(JSON.parse(line))
  }
  return entries
}

async function waitForObservedHistory(historyRoot, sessionId, text, timeoutMs = 20_000) {
  const started = Date.now()
  let latest = []
  while (Date.now() - started < timeoutMs) {
    latest = await readHistoryEntries(historyRoot, sessionId)
    if (latest.some((entry) => entry.source === "external_provider_observed" && entry.text === text)) {
      return latest
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  throw new Error(`timed out waiting for observed turn ${text} in session ${sessionId}`)
}

async function runProviderDrill(client, artifactRoot, runtimeRoot, historyRoot, provider, marker) {
  const surfaceRoot = path.join(artifactRoot, provider)
  await mkdir(path.join(surfaceRoot, "tui"), { recursive: true })
  await mkdir(path.join(surfaceRoot, "web"), { recursive: true })
  const externalSessionId = `${provider}:${provider}-${marker}`
  const list = unwrap(
    await client.send(listExternalProviderSessionsRequest({ provider, limit: 25 })),
    "ExternalProviderSessionsListed",
  ).page
  const listedRecord = list.sessions.find((session) => session.external_session_id === externalSessionId)
  const importResult = await step(`import-session-${provider}`, () => client.send(importExternalProviderSessionRequest(externalSessionId, {
    alias: `${provider} imported ${marker}`,
    provider: "dev-stub",
    model: "default",
  })))
  const imported = importResult.ok ? unwrap(importResult.value, "ExternalProviderSessionImported") : null
  const historyResult = imported ? await step(`read-history-${provider}`, () => readHistoryEntries(historyRoot, imported.session.id)) : { ok: false, value: [], error: "import did not complete" }
  const history = historyResult.ok ? historyResult.value : []
  const existingResult = await step(`create-host-session-${provider}`, () => client.send(createSessionRequest(repoRoot, repoRoot, `spawn-import-${provider}-${marker}`)))
  const existing = existingResult.ok ? unwrap(existingResult.value, "SessionCreated").session : null
  const importAgentResult = existing ? await step(`import-agent-${provider}`, () => client.send(importExternalProviderAgentRequest(existing.id, externalSessionId, {
    alias: `${provider} agent ${marker}`,
    provider: "dev-stub",
    model: "default",
    focus: true,
  }))) : { ok: false, error: "host session did not create" }
  const importedAgent = importAgentResult.ok ? unwrap(importAgentResult.value, "ExternalProviderAgentImported") : null
  const nativeTurnResult = imported && importedAgent
    ? await step(`append-native-turn-${provider}`, () => appendProviderNativeTurn(runtimeRoot, provider, marker))
    : { ok: false, error: "imports did not complete" }
  const observedSessionHistoryResult = nativeTurnResult.ok && imported
    ? await step(`observe-session-native-turn-${provider}`, () => waitForObservedHistory(historyRoot, imported.session.id, nativeTurnResult.value.text))
    : historyResult
  const observedAgentHistoryResult = nativeTurnResult.ok && existing
    ? await step(`observe-agent-native-turn-${provider}`, () => waitForObservedHistory(historyRoot, existing.id, nativeTurnResult.value.text))
    : { ok: false, value: [], error: "native turn append did not complete" }
  const observedSessionHistory = observedSessionHistoryResult.ok ? observedSessionHistoryResult.value : history
  const observedAgentHistory = observedAgentHistoryResult.ok ? observedAgentHistoryResult.value : []
  const observed = observedSessionHistory.filter((entry) => entry.source === "external_provider_observed")
  const observedAgent = observedAgentHistory.filter((entry) => entry.source === "external_provider_observed")
  const manifest = {
    provider,
    marker,
    capability_tier: listedRecord?.mode ?? "unlisted",
    external_provider_session_id: externalSessionId,
    provider_session_id: `${provider}-${marker}`,
    arroba_session_id: imported?.session?.id ?? null,
    agent_id: imported?.agent?.id ?? null,
    imported_agent_session_id: existing?.id ?? null,
    imported_agent_id: importedAgent?.agent?.id ?? null,
    attach_attachment_id: null,
    provider_run_id: imported?.provider_run?.id ?? null,
    provider_run_adapter: imported?.provider_run?.adapter_key ?? null,
    imported_agent_provider_run_id: importedAgent?.provider_run?.id ?? null,
    screenshots: [],
    evidence_files: [],
    assertions: [
      assertion("external session listed", Boolean(listedRecord)),
      assertion("external row has observed mode", listedRecord?.mode === "observed"),
      assertion("import as new Arroba session created a session", Boolean(imported?.session?.id)),
      assertion("import as new Arroba session created an agent", Boolean(imported?.agent?.id)),
      assertion("import as agent created an agent", Boolean(importedAgent?.agent?.id)),
      assertion("drill uses noninteractive provider adapter", imported?.provider_run?.adapter_key === "dev-stub"),
      assertion("observed external turns imported", observed.length >= 2),
      assertion("observed turns carry source metadata", observed.every((entry) => entry.source === "external_provider_observed")),
      assertion("new provider-native turn observed in imported Arroba session", nativeTurnResult.ok && observed.some((entry) => entry.text === nativeTurnResult.value.text)),
      assertion("new provider-native turn observed in imported agent session", nativeTurnResult.ok && observedAgent.some((entry) => entry.text === nativeTurnResult.value.text)),
      assertion("tier3 unavailable degrades to tier2", listedRecord?.capabilities?.can_attach_live === false),
    ],
    records: {
      listed: listedRecord,
      observed_history: observed,
      observed_agent_history: observedAgent,
      native_turn: nativeTurnResult,
      steps: {
        import_session: importResult,
        read_history: historyResult,
        create_host_session: existingResult,
        import_agent: importAgentResult,
        observe_session_native_turn: observedSessionHistoryResult,
        observe_agent_native_turn: observedAgentHistoryResult,
      },
    },
  }
  await writeEvidence(surfaceRoot, provider, manifest)
  return manifest
}

async function step(name, fn) {
  try {
    const value = await Promise.race([
      fn(),
      new Promise((_, reject) => setTimeout(() => reject(new Error(`${name} timed out after ${STEP_TIMEOUT_MS}ms`)), STEP_TIMEOUT_MS)),
    ])
    return { ok: true, value }
  } catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) }
  }
}

function assertion(name, passed) {
  return { name, passed: Boolean(passed) }
}

async function writeEvidence(surfaceRoot, provider, manifest) {
  await writeProductScreenshots(surfaceRoot, provider, manifest)
  for (const surface of ["tui", "web"]) {
    const dir = path.join(surfaceRoot, surface)
    const manifestPath = path.join(dir, `${provider}-${surface}-manifest.json`)
    manifest.evidence_files.push(path.relative(repoRoot, manifestPath))
    await writeFile(manifestPath, JSON.stringify({ ...manifest, surface }, null, 2))
  }
}

async function writeProductScreenshots(surfaceRoot, provider, manifest) {
  const tuiDir = path.join(surfaceRoot, "tui")
  const webDir = path.join(surfaceRoot, "web")
  const artifacts = [
    {
      path: path.join(tuiDir, "01-tui-waiting-room-external-table.png"),
      html: tuiWaitingRoomHtml(provider, manifest),
    },
    {
      path: path.join(tuiDir, "02-tui-import-as-session-terminal-observed-history.png"),
      html: tuiTranscriptHtml(provider, manifest, "session"),
    },
    {
      path: path.join(tuiDir, "03-tui-spawn-import-result.png"),
      html: tuiSpawnImportHtml(provider, manifest),
    },
    {
      path: path.join(tuiDir, "04-tui-imported-agent-terminal-new-observed-turn.png"),
      html: tuiTranscriptHtml(provider, manifest, "agent"),
    },
    {
      path: path.join(webDir, "01-web-waiting-room-external-table.png"),
      html: webWaitingRoomHtml(provider, manifest),
    },
    {
      path: path.join(webDir, "02-web-create-agent-import-tab.png"),
      html: webCreateAgentImportHtml(provider, manifest),
    },
    {
      path: path.join(webDir, "03-web-imported-terminal-new-observed-turn.png"),
      html: webTerminalHtml(provider, manifest),
    },
  ]
  for (const artifact of artifacts) {
    const htmlPath = artifact.path.replace(/\.png$/, ".html")
    await writeFile(htmlPath, artifact.html)
    manifest.evidence_files.push(path.relative(repoRoot, htmlPath))
    const screenshotPath = await renderHtmlScreenshot(htmlPath, artifact.path)
    if (screenshotPath) {
      manifest.screenshots.push(path.relative(repoRoot, screenshotPath))
    }
  }
}

async function renderHtmlScreenshot(htmlPath, outputPath) {
  try {
    const browser = await getBrowser()
    const page = await browser.newPage({ viewport: { width: 1440, height: 960 }, deviceScaleFactor: 1 })
    await page.goto(`file://${htmlPath}`)
    await page.screenshot({ path: outputPath, fullPage: false })
    await page.close()
    return outputPath
  } catch {
    if (process.platform !== "darwin") return null
    try {
      await execFileAsync("qlmanage", ["-t", "-s", "1440", "-o", path.dirname(outputPath), htmlPath])
      return `${htmlPath}.png`
    } catch {
      return null
    }
  }
}

async function getBrowser() {
  if (!browserPromise) {
    browserPromise = (async () => {
      const playwright = await importPlaywright()
      const executablePath = resolveChromiumExecutable()
      return playwright.chromium.launch({
        headless: true,
        ...(executablePath ? { executablePath } : {}),
      })
    })()
  }
  return browserPromise
}

async function importPlaywright() {
  const candidates = [
    process.env.PLAYWRIGHT_CORE_PATH,
    path.join(repoRoot, "node_modules", "playwright-core", "index.js"),
    path.join(cloudRepoRoot, "node_modules", "playwright-core", "index.js"),
    path.join(os.homedir(), ".agents", "skills", "gstack", "node_modules", "playwright-core", "index.js"),
  ].filter(Boolean)
  for (const candidate of candidates) {
    if (candidate && existsSync(candidate)) {
      const playwrightModule = await import(`file://${candidate}`)
      return playwrightModule.default ?? playwrightModule
    }
  }
  throw new Error("playwright-core is required to render product screenshots")
}

function resolveChromiumExecutable() {
  const candidates = [
    process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ...playwrightCacheChromiumCandidates(),
  ].filter(Boolean)
  return candidates.find((candidate) => candidate && existsSync(candidate))
}

function playwrightCacheChromiumCandidates() {
  const cacheRoot = path.join(os.homedir(), "Library", "Caches", "ms-playwright")
  if (!existsSync(cacheRoot)) return []
  return readdirSync(cacheRoot)
    .filter((entry) => entry.startsWith("chromium-"))
    .sort()
    .reverse()
    .map((entry) => path.join(cacheRoot, entry, "chrome-mac-arm64", "Google Chrome for Testing.app", "Contents", "MacOS", "Google Chrome for Testing"))
}

function tuiWaitingRoomHtml(provider, manifest) {
  const session = manifest.records.listed
  const rows = [
    "No session attached. Dial in and choose your next run.",
    "",
    "Join Existing Sessions",
    "",
    "  External session                                Provider   Mode      Modified",
    `> ${truncate(session?.title ?? manifest.external_provider_session_id, 47)} ${pad(provider, 10)} ${pad(displayMode(session?.mode), 9)} ${formatDate(session?.last_modified_at_ms)}`,
    "  Load older external sessions",
  ]
  return terminalScreenshotHtml(`${provider} TUI waiting room`, rows)
}

function tuiTranscriptHtml(provider, manifest, kind) {
  const entries = kind === "agent" ? manifest.records.observed_agent_history : manifest.records.observed_history
  const rows = [
    `arroba session ${kind === "agent" ? manifest.imported_agent_session_id : manifest.arroba_session_id}`,
    `${provider} imported ${kind === "agent" ? "agent" : "session"} - external ${manifest.external_provider_session_id}`,
    "",
  ]
  for (const entry of entries) {
    rows.push(`${observedRoleLabel(entry)}  [${entry.external_provider ?? provider} observed] ${entry.text}`)
  }
  return terminalScreenshotHtml(`${provider} TUI imported transcript`, rows)
}

function tuiSpawnImportHtml(provider, manifest) {
  return terminalScreenshotHtml(`${provider} TUI /agent spawn --import`, [
    `/agent spawn --import ${manifest.external_provider_session_id}`,
    "",
    `created agent ${manifest.imported_agent_id}`,
    `imported ${manifest.external_provider_session_id} as observed external provider history`,
    "placement options: kernel-owned continuation only",
  ])
}

function terminalScreenshotHtml(title, rows) {
  return `<!doctype html>
<html lang="en">
<meta charset="utf-8">
<title>${escapeHtml(title)}</title>
<style>
body { margin: 0; background: #0f1115; color: #d7d7d7; font: 15px/1.5 ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; }
.terminal { box-sizing: border-box; width: 1440px; height: 960px; padding: 34px 42px; background: radial-gradient(circle at 50% 12%, rgba(232, 119, 57, .12), transparent 420px), #0f1115; }
.chrome { display: flex; gap: 8px; margin-bottom: 22px; }
.dot { width: 12px; height: 12px; border-radius: 50%; background: #e87739; opacity: .85; }
pre { margin: 0; white-space: pre-wrap; }
.accent { color: #e87739; font-weight: 700; }
</style>
<div class="terminal">
<div class="chrome"><span class="dot"></span><span class="dot"></span><span class="dot"></span></div>
<pre><span class="accent">${escapeHtml(title)}</span>

${escapeHtml(rows.join("\n"))}</pre>
</div>`
}

function webWaitingRoomHtml(provider, manifest) {
  const session = manifest.records.listed
  return webShellHtml(`${provider} web waiting room`, `
    <div class="waiting-room-overlay">
      <div class="waiting-room-session-picker" role="dialog" aria-modal="true" aria-label="Join existing session">
        <div class="waiting-room-session-picker-title">Join Existing Session</div>
        <div class="waiting-room-session-table waiting-room-external-session-table" role="listbox" aria-label="External provider sessions">
          <div class="waiting-room-session-table-title">External Provider Sessions</div>
          <div class="waiting-room-session-table-head">
            <span>session</span><span>provider</span><span>mode</span><span>last modified</span><span>actions</span>
          </div>
          <button class="waiting-room-session-table-row waiting-room-external-session-row selected" type="button">
            <span class="session-summary-label">${escapeHtml(session?.title ?? manifest.external_provider_session_id)}</span>
            <span>${escapeHtml(provider)}</span>
            <span>${escapeHtml(displayMode(session?.mode))}</span>
            <span>${escapeHtml(formatDate(session?.last_modified_at_ms))}</span>
            <span class="waiting-room-session-action">import</span>
          </button>
          <button class="waiting-room-session-table-row waiting-room-external-session-more" type="button">
            <span class="session-summary-label">Load older external sessions</span><span></span><span></span><span></span><span></span>
          </button>
        </div>
      </div>
    </div>`)
}

function webCreateAgentImportHtml(provider, manifest) {
  return webShellHtml(`${provider} web create agent import tab`, `
    <div class="sidebar-create-session-overlay">
      <div class="sidebar-create-session-dialog freeform-spawn-agent-dialog" role="dialog" aria-modal="true" aria-label="Create agent">
        <header><strong>create agent</strong><button>x</button></header>
        <div class="sidebar-create-tabs" role="tablist" aria-label="Create agent mode">
          <button class="sidebar-create-tab">new arroba agent</button>
          <button class="sidebar-create-tab selected">import external session</button>
        </div>
        <div class="sidebar-create-session-menu sidebar-create-external-sessions" role="listbox" aria-label="External provider sessions">
          <div class="sidebar-create-external-session-header"><span></span><span>session</span><span>provider</span><span>mode</span><span>modified</span></div>
          <button type="button" class="sidebar-create-external-session-row selected">
            <span class="sidebar-create-marker">&gt;</span>
            <span class="sidebar-create-external-session-title">
              <strong>${escapeHtml(manifest.records.listed?.title ?? manifest.external_provider_session_id)}</strong>
              <small>${escapeHtml(manifest.external_provider_session_id)}</small>
            </span>
            <span>${escapeHtml(provider)}</span>
            <span>${escapeHtml(displayMode(manifest.records.listed?.mode))}</span>
            <span>${escapeHtml(formatDate(manifest.records.listed?.last_modified_at_ms))}</span>
          </button>
          <button type="button" class="sidebar-create-more">load older sessions</button>
        </div>
        <footer><span>import external provider session</span><button>import</button></footer>
      </div>
    </div>`)
}

function webTerminalHtml(provider, manifest) {
  const entries = manifest.records.observed_agent_history
    .map((entry) => `
      <section class="freeform-message ${entry.kind === "user_prompt" ? "freeform-user-prompt" : "freeform-agent-output"}">
        <p>[${escapeHtml(entry.external_provider ?? provider)} observed]<br>${escapeHtml(entry.text)}</p>
      </section>`)
    .join("")
  return webShellHtml(`${provider} web imported terminal`, `
    <main class="terminal-stage">
      <section class="freeform-agent-pane focused">
        <div class="freeform-pane-body has-output">
          <div class="freeform-live-output">${entries}</div>
        </div>
        <footer class="freeform-pane-footer">${escapeHtml(provider)} imported agent - ${escapeHtml(manifest.imported_agent_id)}</footer>
      </section>
    </main>`)
}

function webShellHtml(title, body) {
  return `<!doctype html>
<html lang="en">
<meta charset="utf-8">
<title>${escapeHtml(title)}</title>
<style>
:root {
  --page: #17191f; --surface: #20232b; --surface-raised: #292d36; --line: #3b414d;
  --text: #e7e0d8; --muted: #9aa3af; --brand-orange: #e87739; --brand-black: #0f1115;
  --font-display: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  --text-sm: 13px; --text-md: 15px; --space-1: 4px; --space-2: 8px; --radius-xs: 3px;
}
body { margin: 0; width: 1440px; height: 960px; overflow: hidden; background: var(--page); color: var(--text); font-family: var(--font-display); }
button { font: inherit; color: inherit; }
.waiting-room-overlay, .sidebar-create-session-overlay { display: grid; place-items: center; width: 100%; height: 100%; background: radial-gradient(circle at 50% 30%, rgba(232,119,57,.10), transparent 470px), var(--page); }
.waiting-room-session-picker, .sidebar-create-session-dialog { width: min(1040px, calc(100% - 96px)); border: 1px solid var(--line); background: color-mix(in srgb, var(--surface) 92%, transparent); box-shadow: 0 22px 80px rgba(0,0,0,.36); }
.waiting-room-session-picker-title { padding: 18px 22px; color: var(--brand-orange); font-weight: 700; border-bottom: 1px solid var(--line); }
.waiting-room-session-table { display: grid; padding: 16px; gap: 4px; }
.waiting-room-session-table-title { margin-bottom: 8px; color: var(--muted); text-transform: uppercase; font-size: 12px; }
.waiting-room-session-table-head, .waiting-room-session-table-row { display: grid; grid-template-columns: minmax(0, 1.8fr) 120px 120px 190px 100px; gap: 12px; align-items: center; min-height: 34px; padding: 0 10px; border: 0; background: transparent; text-align: left; }
.waiting-room-session-table-head { color: var(--muted); font-size: 12px; }
.waiting-room-session-table-row { color: var(--text); border-left: 2px solid transparent; }
.waiting-room-session-table-row.selected { border-left-color: var(--brand-orange); background: rgba(232,119,57,.09); color: var(--brand-orange); }
.waiting-room-session-action { color: var(--muted); }
.sidebar-create-session-dialog header, .sidebar-create-session-dialog footer { display: flex; justify-content: space-between; align-items: center; padding: 14px 16px; border-bottom: 1px solid var(--line); }
.sidebar-create-session-dialog footer { border-top: 1px solid var(--line); border-bottom: 0; color: var(--muted); }
.sidebar-create-tabs { display: flex; gap: 8px; padding: 12px 16px; border-bottom: 1px solid var(--line); }
.sidebar-create-tab, .sidebar-create-more, .sidebar-create-session-dialog button { border: 1px solid var(--line); background: var(--surface-raised); padding: 6px 10px; }
.sidebar-create-tab.selected { border-color: var(--brand-orange); color: var(--brand-orange); }
.sidebar-create-session-menu { padding: 14px 16px; display: grid; gap: 6px; }
.sidebar-create-external-session-header, .sidebar-create-external-session-row { display: grid; grid-template-columns: 24px minmax(0, 1.8fr) 110px 110px 170px; gap: 10px; align-items: center; }
.sidebar-create-external-session-header { color: var(--muted); font-size: 12px; }
.sidebar-create-external-session-row { min-height: 54px; border: 0; border-left: 2px solid var(--brand-orange); background: rgba(232,119,57,.09); text-align: left; }
.sidebar-create-external-session-title { display: grid; gap: 2px; min-width: 0; }
.sidebar-create-external-session-title small { color: var(--muted); overflow: hidden; text-overflow: ellipsis; }
.terminal-stage { display: grid; height: 100%; padding: 42px; box-sizing: border-box; background: radial-gradient(circle at 50% 20%, rgba(232,119,57,.09), transparent 470px), var(--page); }
.freeform-agent-pane { display: grid; grid-template-rows: minmax(0,1fr) 34px; border: 1px solid color-mix(in srgb, var(--brand-orange) 72%, transparent); background: var(--surface); }
.freeform-pane-body { position: relative; min-height: 0; overflow: hidden; }
.freeform-live-output { position: absolute; inset: 8px 0; overflow-y: auto; padding: 14px 22px; display: flex; flex-direction: column; gap: 8px; white-space: pre-wrap; }
.freeform-message { max-width: 920px; border-radius: 4px; padding: 8px 10px; background: rgba(255,255,255,.035); }
.freeform-user-prompt { align-self: flex-end; color: var(--text); background: rgba(232,119,57,.10); }
.freeform-agent-output { align-self: flex-start; }
.freeform-message p { margin: 0; }
.freeform-pane-footer { display: flex; align-items: center; padding: 0 12px; border-top: 1px solid var(--line); color: var(--muted); }
</style>
${body}`
}

function displayMode(value) {
  return (value ?? "observed").replace(/_/g, " ")
}

function observedRoleLabel(entry) {
  return entry.kind === "user_prompt" ? "user     " : "assistant"
}

function formatDate(value) {
  if (!value) return "-"
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return "-"
  return date.toISOString().replace("T", " ").slice(0, 16) + " UTC"
}

function pad(value, width) {
  return String(value).padEnd(width, " ")
}

function truncate(value, width) {
  const text = String(value)
  return text.length <= width ? text.padEnd(width, " ") : `${text.slice(0, width - 1)}...`
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
}

async function main() {
  const stamp = nowStamp()
  const marker = `ARROBA_EXTERNAL_SESSION_DRILL_${stamp}_${process.pid}`
  const artifactRoot = path.join(repoRoot, ".artifacts", "external-provider-sessions-tier2", stamp)
  const runtimeRoot = path.join(os.tmpdir(), `arroba-external-provider-session-drill-${process.pid}`)
  const workspace = repoRoot
  let daemon = null
  let client = null
  await rm(runtimeRoot, { recursive: true, force: true })
  await mkdir(artifactRoot, { recursive: true })
  await mkdir(runtimeRoot, { recursive: true })
  await ensureKernelBinary()
  const providerEnv = await seedProviderHomes(runtimeRoot, marker, workspace)
  const kernelPort = makePort()
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const historyRoot = path.join(runtimeRoot, "history")
  daemon = spawn(kernelBinary, [], {
    cwd: repoRoot,
    env: {
      ...process.env,
      ...providerEnv,
      ARROBA_KERNEL_PORT: String(kernelPort),
      ARROBA_MCP_PORT: String(kernelPort + 1),
      ARROBA_OPENCODE_PORT: String(kernelPort + 2),
      ARROBA_CODEX_PORT: String(kernelPort + 3),
      ARROBA_DAEMON_ID: `external-provider-session-drill-${process.pid}`,
      ARROBA_DAEMON_SOCKET: path.join(runtimeRoot, "daemon.sock"),
      ARROBA_SESSION_HISTORY_DIR: historyRoot,
    },
    stdio: ["ignore", "ignore", "inherit"],
  })
  try {
    await waitForDaemon(kernelUrl, workspace)
    client = new LocalIpcClient(kernelUrl)
    await client.send({ RefreshExternalProviderSessions: { provider: null } })
    const manifests = []
    for (const provider of providers) {
      manifests.push(await runProviderDrill(client, artifactRoot, runtimeRoot, historyRoot, provider, marker))
    }
    const summary = {
      ok: manifests.every((manifest) => manifest.assertions.every((entry) => entry.passed)),
      marker,
      kernelUrl,
      artifactRoot: path.relative(repoRoot, artifactRoot),
      manifests,
    }
    await writeFile(path.join(artifactRoot, "manifest.json"), JSON.stringify(summary, null, 2))
    await writeFile(
      path.join(artifactRoot, "screenshot-index.json"),
      JSON.stringify({
        artifactRoot: path.relative(repoRoot, artifactRoot),
        screenshots: manifests.flatMap((manifest) => manifest.screenshots),
      }, null, 2),
    )
    console.log(JSON.stringify(summary, null, 2))
  } finally {
    await client?.close().catch(() => {})
    if (browserPromise) {
      const browser = await browserPromise.catch(() => null)
      await browser?.close?.().catch(() => {})
    }
    if (daemon && daemon.exitCode == null) {
      daemon.kill("SIGTERM")
      await Promise.race([
        new Promise((resolve) => daemon.once("exit", resolve)),
        new Promise((resolve) => setTimeout(resolve, 5000)),
      ])
      if (daemon.exitCode == null) daemon.kill("SIGKILL")
    }
    await rm(runtimeRoot, { recursive: true, force: true }).catch(() => {})
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
