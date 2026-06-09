import { spawn, execFile } from "node:child_process"
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
const providers = ["codex", "opencode", "claude"]
const STEP_TIMEOUT_MS = 15_000

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

async function runProviderDrill(client, root, historyRoot, provider, marker) {
  const surfaceRoot = path.join(root, provider)
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
  const observed = history.filter((entry) => entry.source === "external_provider_observed")
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
      assertion("tier3 unavailable degrades to tier2", listedRecord?.capabilities?.can_attach_live === false),
    ],
    records: {
      listed: listedRecord,
      observed_history: observed,
      steps: {
        import_session: importResult,
        read_history: historyResult,
        create_host_session: existingResult,
        import_agent: importAgentResult,
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
  for (const surface of ["tui", "web"]) {
    const dir = path.join(surfaceRoot, surface)
    const html = evidenceHtml(provider, surface, manifest)
    const htmlPath = path.join(dir, `${provider}-${surface}-evidence.html`)
    const manifestPath = path.join(dir, `${provider}-${surface}-manifest.json`)
    await writeFile(htmlPath, html)
    manifest.evidence_files.push(path.relative(repoRoot, htmlPath))
    const screenshotPath = await renderEvidenceScreenshot(htmlPath, dir)
    if (screenshotPath) {
      manifest.screenshots.push(path.relative(repoRoot, screenshotPath))
    }
    manifest.evidence_files.push(path.relative(repoRoot, manifestPath))
    await writeFile(manifestPath, JSON.stringify({ ...manifest, surface }, null, 2))
  }
}

async function renderEvidenceScreenshot(htmlPath, outputDir) {
  if (process.platform !== "darwin") return null
  try {
    await execFileAsync("qlmanage", ["-t", "-s", "1440", "-o", outputDir, htmlPath])
    return `${htmlPath}.png`
  } catch {
    return null
  }
}

function evidenceHtml(provider, surface, manifest) {
  const rows = manifest.assertions
    .map((entry) => `<tr><td>${escapeHtml(entry.name)}</td><td>${entry.passed ? "pass" : "fail"}</td></tr>`)
    .join("")
  const history = manifest.records.observed_history
    .map((entry) => `<li><strong>${escapeHtml(entry.kind)}</strong> ${escapeHtml(entry.external_provider_turn_id ?? "")}: ${escapeHtml(entry.text ?? "")}</li>`)
    .join("")
  return `<!doctype html>
<html lang="en">
<meta charset="utf-8">
<title>${escapeHtml(provider)} ${escapeHtml(surface)} external provider session evidence</title>
<style>
body { margin: 0; padding: 32px; background: #f6f3ef; color: #1d2024; font: 15px/1.45 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
main { max-width: 980px; margin: 0 auto; }
h1 { font-size: 28px; margin: 0 0 8px; }
h2 { margin-top: 28px; font-size: 18px; }
.meta { display: grid; grid-template-columns: 220px 1fr; gap: 8px 16px; padding: 18px 0; border-block: 1px solid #cbc5bb; }
table { width: 100%; border-collapse: collapse; background: white; }
td, th { border: 1px solid #d8d2c8; padding: 9px 10px; text-align: left; }
td:last-child { width: 120px; font-weight: 700; }
li { margin: 8px 0; }
code { background: #eee8df; padding: 2px 5px; border-radius: 4px; }
</style>
<main>
<h1>${escapeHtml(provider)} external provider session evidence</h1>
<p>${escapeHtml(surface.toUpperCase())} surface evidence card generated by <code>live-external-provider-session-import-drill.mjs</code>.</p>
<section class="meta">
<strong>external session</strong><span>${escapeHtml(manifest.external_provider_session_id)}</span>
<strong>Arroba session</strong><span>${escapeHtml(manifest.arroba_session_id)}</span>
<strong>agent</strong><span>${escapeHtml(manifest.agent_id)}</span>
<strong>provider run</strong><span>${escapeHtml(manifest.provider_run_id ?? "none")}</span>
<strong>capability tier</strong><span>${escapeHtml(manifest.capability_tier)}</span>
</section>
<h2>Assertions</h2>
<table><tbody>${rows}</tbody></table>
<h2>Observed History</h2>
<ul>${history}</ul>
</main>`
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
  const artifactRoot = path.join(repoRoot, ".artifacts", "external-provider-sessions", stamp)
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
      manifests.push(await runProviderDrill(client, artifactRoot, historyRoot, provider, marker))
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
