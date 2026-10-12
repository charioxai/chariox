// MP-08 / MP-10 / MP-11: inspect the kernel-owned native run, never a CLI temp root.
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
  return { client, sessionId: run.session_id, agentId: run.agent_instance_id }
}

// Read the same protected user-visible history used by clients. GetProviderRun
// intentionally excludes provider environments and private hook mailbox paths.
export async function waitForClaudeNativePrompts(source, expected, timeoutMs = 120_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const response = await source.client.send({ GetSessionHistoryOutline: {
      session_id: source.sessionId, agent_ids: [source.agentId], latest_prompt_count: 100,
    } })
    const prompts = (response.SessionHistoryOutline?.agents ?? [])
      .flatMap(agent => agent.turns ?? []).map(turn => turn.user_prompt?.entry?.text ?? "")
    if (expected.every(value => prompts.some(prompt => prompt.includes(value)))) return prompts
    await sleep(250)
  }
  throw new Error("Claude kernel history did not contain every expected native prompt")
}

export async function captureClaudeNativeScreen(screen, file) {
  await promisify(execFile)("screen", ["-S", screen, "-X", "hardcopy", "-h", file])
}
