// MP-08 / MP-10 / MP-11: inspect the kernel-owned native run, never a CLI temp root.
import path from "node:path"
import { readBoundedPrivateInput, requireScopedProviderPath } from "./private-drill-runtime.mjs"
import { execFile } from "node:child_process"
import { readFile } from "node:fs/promises"
import { setTimeout as sleep } from "node:timers/promises"
import { promisify } from "node:util"

export async function claudeNativeRunPaths(client, bannerFile) {
  const banner = await readFile(bannerFile, "utf8")
  const runId = /provider run:\s+([^\s]+)/.exec(banner)?.[1]
  if (!runId) throw new Error("Claude kernel provider run missing from banner")
  const response = await client.send({ GetProviderRun: { provider_run_id: runId } })
  const run = response.ProviderRun?.provider_run
  if (!run?.session_id || !run?.agent_instance_id) throw new Error("Claude kernel run has no session/agent binding")
  return { client, runId, sessionId: run.session_id, agentId: run.agent_instance_id }
}

// Read the same protected user-visible history used by clients. GetProviderRun
// intentionally excludes provider environments and private hook mailbox paths.
export async function waitForClaudeNativePrompts(source, expected, timeoutMs = 120_000, { providerTranscript = false } = {}) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (providerTranscript) {
      const prompts = await claudeTranscriptPrompts(source)
      if (expected.every(value => prompts.some(prompt => prompt.includes(value)))) return prompts
      await sleep(250)
      continue
    }
    const response = await source.client.send({ GetSessionHistoryOutline: {
      session_id: source.sessionId, agent_ids: [source.agentId], latest_prompt_count: 100,
    } })
    const prompts = (response.SessionHistoryOutline?.agents ?? [])
      .flatMap(agent => agent.turns ?? []).map(turn => turn.user_prompt?.entry?.text ?? "")
    if (expected.every(value => prompts.some(prompt => prompt.includes(value)))) return prompts
    await sleep(250)
  }
  throw new Error(`Claude ${providerTranscript ? "provider transcript" : "kernel history"} did not contain every expected native prompt`)
}

export async function captureClaudeNativeScreen(screen, file) {
  await promisify(execFile)("screen", ["-S", screen, "-X", "hardcopy", "-h", file])
}

// MP-08 / MP-10 / MP-11: hidden-context checks must inspect the official
// provider's visible user messages, independently of Chariox's clean history.
async function claudeTranscriptPrompts(source) {
  const configDir = requireScopedProviderPath(process.env, "CLAUDE_CONFIG_DIR")
  const response = await source.client.send({ GetProviderRun: { provider_run_id: source.runId } })
  const run = response.ProviderRun?.provider_run
  if (run?.session_id !== source.sessionId || run?.agent_instance_id !== source.agentId) {
    throw new Error("Claude transcript run lost its session/agent binding")
  }
  if (!run.provider_session_id) return []
  if (!/^[a-zA-Z0-9_-]+$/.test(run.provider_session_id) || !path.isAbsolute(run.working_directory ?? "")) {
    throw new Error("Claude public run has no safe transcript identity")
  }
  const project = run.working_directory.replace(/[^a-zA-Z0-9]/g, "-")
  const file = path.join(configDir, "projects", project, `${run.provider_session_id}.jsonl`)
  let raw
  try { raw = await readBoundedPrivateInput(file) }
  catch (error) { if (error.code === "ENOENT") return []; throw error }
  return raw.split("\n").filter(Boolean).flatMap(line => {
    let row
    try { row = JSON.parse(line) } catch { return [] }
    if (row.type !== "user" || row.isMeta || row.isCompactSummary) return []
    const content = row.message?.content
    if (typeof content === "string") return [content]
    return (Array.isArray(content) ? content : [])
      .filter(block => block.type === "text" && typeof block.text === "string")
      .map(block => block.text)
  })
}
