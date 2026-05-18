import { writeFile } from "node:fs/promises"

export async function writeClaudeHookHandler(file: string) {
  await writeFile(file, `#!/usr/bin/env node
import { appendFileSync, existsSync, readFileSync, unlinkSync } from "node:fs"
import { join } from "node:path"
import { setTimeout as sleep } from "node:timers/promises"

const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
const raw = Buffer.concat(chunks).toString("utf8")
let input = {}
try {
  input = raw.trim() ? JSON.parse(raw) : {}
} catch (error) {
  input = { hook_event_name: "parse_error", raw, error: String(error) }
}
const eventName = input.hook_event_name ?? "unknown"
const hookContextRequestId = eventName === "UserPromptSubmit"
  ? \`\${Date.now()}-\${process.pid}-\${Math.random().toString(36).slice(2)}\`
  : null
appendFileSync(process.env.ARROBA_CLAUDE_NATIVE_EVENTS, JSON.stringify({
  at: new Date().toISOString(),
  hook_event_name: eventName,
  hook_context_request_id: hookContextRequestId,
  prompt: input.prompt ?? null,
  transcript_path: input.transcript_path ?? null,
  permission_mode: input.permission_mode ?? null,
  tool_name: input.tool_name ?? null,
  tool_input: input.tool_input ?? null,
  tool_response: input.tool_response ?? null,
  error: input.error ?? null,
}) + "\\n")

if (eventName === "UserPromptSubmit") {
  let additionalContext = ""
  try {
    additionalContext = readFileSync(process.env.ARROBA_CLAUDE_NATIVE_CONTEXT, "utf8")
  } catch {}
  if (!additionalContext && hookContextRequestId && process.env.ARROBA_CLAUDE_NATIVE_CONTEXT_RESPONSES) {
    const responseFile = join(process.env.ARROBA_CLAUDE_NATIVE_CONTEXT_RESPONSES, \`\${hookContextRequestId}.txt\`)
    const deadline = Date.now() + 5000
    while (Date.now() < deadline) {
      if (existsSync(responseFile)) {
        additionalContext = readFileSync(responseFile, "utf8")
        try { unlinkSync(responseFile) } catch {}
        break
      }
      await sleep(50)
    }
  }
  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: "UserPromptSubmit",
      additionalContext
    }
  }))
} else if (eventName === "PreToolUse" || eventName === "PermissionRequest") {
  const bridgeUrl = process.env.ARROBA_CLAUDE_NATIVE_HOOK_BRIDGE_URL
  if (bridgeUrl) {
    try {
      const response = await fetch(new URL("/permission", bridgeUrl), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(input)
      })
      if (response.ok) {
        const decision = await response.json()
        if (decision?.handled && decision.permissionDecision) {
          process.stdout.write(JSON.stringify({
            hookSpecificOutput: {
              hookEventName: eventName,
              permissionDecision: decision.permissionDecision,
              permissionDecisionReason: decision.permissionDecisionReason ?? "Resolved through Arroba."
            }
          }))
        }
      }
    } catch {}
  }
}
`, "utf8")
}

export function claudeHookSettings(handlerPath: string) {
  const command = `node ${JSON.stringify(handlerPath)}`
  return {
    hooks: {
      UserPromptSubmit: [{ hooks: [{ type: "command", command }] }],
      Stop: [{ hooks: [{ type: "command", command }] }],
      StopFailure: [{ hooks: [{ type: "command", command }] }],
      SessionEnd: [{ hooks: [{ type: "command", command }] }],
      PermissionRequest: [{ matcher: "*", hooks: [{ type: "command", command }] }],
      PreToolUse: [{ matcher: "*", hooks: [{ type: "command", command }] }],
      PostToolUse: [{ matcher: "*", hooks: [{ type: "command", command }] }],
    },
  }
}
