// MP-08 / MP-10 / MP-11: inspect the kernel-owned native run, never a CLI temp root.
import { execFile } from "node:child_process"
import { readFile } from "node:fs/promises"
import path from "node:path"
import { setTimeout as sleep } from "node:timers/promises"
import { promisify } from "node:util"

export async function claudeNativeRunPaths(client, bannerFile) {
  const banner = await readFile(bannerFile, "utf8")
  const runId = /provider run:\s+([^\s]+)/.exec(banner)?.[1]
  if (!runId) throw new Error("Claude kernel provider run missing from banner")
  const response = await client.send({ GetProviderRun: { provider_run_id: runId } })
  const run = response.ProviderRun?.provider_run
  const events = run?.pty_env?.CHARIOX_CLAUDE_NATIVE_EVENTS
  const context = run?.pty_env?.CHARIOX_CLAUDE_NATIVE_CONTEXT
  if (!events || !context) throw new Error("Claude kernel hook paths missing from provider run")
  return { events, cursor: path.join(path.dirname(context), "transcript-cursor.json") }
}

// events.jsonl is a consumed mailbox. The bridge's cursor names the actual
// official transcript; only its visible user prompts are inspected here.
export async function waitForClaudeNativePrompts(paths, expected, timeoutMs = 120_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const cursor = JSON.parse(await readFile(paths.cursor, "utf8").catch(() => "{}"))
    const prompts = []
    for (const transcript of Object.keys(cursor.files ?? {})) {
      const raw = await readFile(transcript, "utf8").catch(() => "")
      for (const line of raw.split("\n").filter(Boolean)) {
        let row
        try { row = JSON.parse(line) } catch { continue }
        if (row.type !== "user") continue
        const content = row.message?.content
        if (typeof content === "string") prompts.push(content)
        else for (const block of content ?? []) if (block.type === "text") prompts.push(block.text)
      }
    }
    if (expected.every(value => prompts.some(prompt => prompt.includes(value)))) return prompts
    await sleep(250)
  }
  throw new Error("Claude kernel-owned transcript did not contain every expected native prompt")
}

export async function captureClaudeNativeScreen(screen, file) {
  await promisify(execFile)("screen", ["-S", screen, "-X", "hardcopy", "-h", file])
}
