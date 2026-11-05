// Served viewer regression: real gateway/client and bundle, protocol-shaped kernel replies.
// Run after `pnpm --filter @chariox/server build`, with PLAYWRIGHT_MODULE pointing to an installed playwright-core.
import assert from "node:assert/strict"
import { mkdir } from "node:fs/promises"
import path from "node:path"
import test from "node:test"
import { AgentTriggerClient } from "@chariox/agent-trigger-gateway"
import { buildAgentTriggerServer } from "@chariox/agent-trigger-gateway/server"

assert.ok(process.env.PLAYWRIGHT_MODULE, "set PLAYWRIGHT_MODULE to an installed module")
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE)
const firstPng = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAFklEQVR4nGP8z8DAwMDAxMDAwMDAAAANHQEDasKb6QAAAABJRU5ErkJggg==", "base64")
const revisedPng = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1cAAAAASUVORK5CYII=", "base64")
const row = (entry_index, kind, text) => ({ entry_index, total_chars: text.length, entry: { kind, text, timestamp_ms: entry_index } })
const turn = n => ({ turn_id: `turn-${n}`, prompt_id: `prompt-${n}`, started_at_ms: n * 10, completed_at_ms: n * 10 + 1, lifecycle: "completed",
  user_prompt: row(n * 2, "user_prompt", `Draw chart ${n}`), entries: [], blobs: [], summary: row(n * 2 + 1, "provider_output", `Chart ${n}:\n\n![cost](charts/cost.png)`) })

async function fixture() {
  let png = firstPng, turns = [turn(1)], notify = () => {}, reads = 0
  const kernel = {
    async send(request) {
      if (request.ReadAgentTriggerTranscript) return { AgentTriggerTranscript: { trigger: { id: "trigger", agent_id: "agent", enabled: true, source: { kind: "http", ingress_id: "chart" } }, agent_alias: "Chart agent", online: true, busy: false, deliveries: [], history: { SessionHistoryOutline: { agents: [{ agent_id: "agent", turns, next_cursor: null }] } } } }
      if (request.GetSessionState) return { SessionState: { session: { workspace_id: "/work/repo", worktree_id: "/work/repo", agents: [{ id: "agent", workspace_id: null, worktree_id: "/work/repo-wt" }] } } }
      if (request.GetWorkspaceFileContent) {
        assert.deepEqual([request.GetWorkspaceFileContent.workspace_id, request.GetWorkspaceFileContent.worktree_id, request.GetWorkspaceFileContent.path], ["/work/repo", "/work/repo-wt", "charts/cost.png"])
        reads++
        return { WorkspaceFileContent: { content: { content_base64: png.toString("base64"), truncated: false, size_bytes: png.length } } }
      }
      throw Error("unexpected request")
    },
    onKernelEvent(callback) { notify = callback; return () => {} },
    async subscribeToKernelEvents() {}, async close() {},
  }
  const app = await buildAgentTriggerServer(new AgentTriggerClient("session", "trigger", kernel), "127.0.0.1")
  const origin = await app.listen({ host: "127.0.0.1", port: 0 })
  return { app, origin, get reads() { return reads }, refresh() { notify({ event: "session_snapshot" }) }, revise() { png = revisedPng; turns = [...turns, turn(2)]; notify({ event: "assistant_message_completed" }) } }
}
async function withViewer(fn, name) {
  const f = await fixture()
  let browser
  try {
    browser = await chromium.launch({ executablePath: process.env.CHROME_PATH ?? "/opt/google/chrome/chrome", args: ["--no-sandbox", "--disable-dev-shm-usage"] })
    const page = await browser.newPage()
    page.setDefaultTimeout(5_000)
    await page.addInitScript(() => {
      window.revokedImages = []
      const revoke = URL.revokeObjectURL.bind(URL)
      URL.revokeObjectURL = src => { window.revokedImages.push(src); revoke(src) }
    })
    await page.goto(`${f.origin}/chart/`)
    try { await fn(f, page) } finally {
      if (process.env.CHARIOX_VIEWER_EVIDENCE) {
        await mkdir(process.env.CHARIOX_VIEWER_EVIDENCE, { recursive: true })
        await page.screenshot({ path: path.join(process.env.CHARIOX_VIEWER_EVIDENCE, `${name}.png`) })
      }
    }
  } finally { await browser?.close(); await f.app.close() }
}

test("a protocol-shaped session loads a workspace PNG through the served conversation viewer", () => withViewer(async (f, page) => {
  await page.waitForFunction(() => document.querySelector('[data-md-image="workspace"] img')?.naturalWidth === 2)
  assert.equal(f.reads, 1)
}, "workspace-image"))

