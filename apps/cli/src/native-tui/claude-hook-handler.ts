import { writeFile } from "node:fs/promises"

import { claudeHookCommand, releaseVersion } from "../release-build.js"
import { shellQuote } from "./launch-environment.js"

export async function writeClaudeHookHandler(file: string) {
  await writeFile(file, `#!/usr/bin/env node
import { appendFileSync, existsSync, readFileSync, unlinkSync } from "node:fs"
import { join } from "node:path"
import { setTimeout as sleep } from "node:timers/promises"

// MP-08 / MP-10 / MP-11: timeout guard only; the kernel owns resolution.
function hasExplicitSkillMarker(prompt) {
  let fence = null, span = 0
  for (const line of String(prompt).split(/(?<=\\n)/)) {
    const trimmed = line.replace(/^ */, ""), indent = line.length - trimmed.length
    const marker = trimmed[0], count = (trimmed.match(/^(\`+|~+)/)?.[0] ?? "").length
    if (!span && indent <= 3 && count >= 3 && (marker === "\`" || marker === "~")) {
      if (fence) { if (marker === fence[0] && count >= fence[1] && !trimmed.slice(count).trim()) fence = null }
      else fence = [marker, count]
      continue
    }
    if (fence || (!span && indent >= 4)) continue
    let i = 0
    while (i < line.length) {
      if (span) {
        if (line[i] === "\`") { const start = i; while (line[i] === "\`") i++; if (i-start === span) span = 0 }
        else i++
        continue
      }
      if (line[i] === "[") {
        const close = line.indexOf("](", i), end = close < 0 ? -1 : line.indexOf(")", close+2)
        if (end >= 0) { i = end+1; continue }
      }
      if (line[i] === "\\\\") { i += 2; continue }
      if (line[i] === "\`") { const start = i; while (line[i] === "\`") i++; span = i-start; continue }
      if ((line[i] === "'" || line[i] === '"') && !(i > 0 && /[a-zA-Z0-9]/.test(line[i-1]))) {
        const quote = line[i++]
        while (i < line.length) { if (line[i] === "\\\\") i += 2; else if (line[i++] === quote) break }
        continue
      }
      if (line[i] !== "$") { i++; continue }
      const start = i++
      if (start > 0 && !/[\\s([]/.test(line[start-1])) continue
      const nameStart = i
      while (i < line.length && /[a-zA-Z0-9_-]/.test(line[i])) i++
      const name = line.slice(nameStart, i)
      if (!/^[a-zA-Z][a-zA-Z0-9_-]{0,127}$/.test(name) || /^[A-Z0-9_]+$/.test(name)) continue
      if (i < line.length && !/[\\s)\\],.!?;:]/.test(line[i])) continue
      if (line[i] === "]" && line[i+1] === "(") continue
      return true
    }
  }
  return false
}

let nativeOrigin = null
try { nativeOrigin = JSON.parse(readFileSync(process.env.CHARIOX_CLAUDE_NATIVE_ORIGIN, "utf8")) } catch {}
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
appendFileSync(process.env.CHARIOX_CLAUDE_NATIVE_EVENTS, JSON.stringify({
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
    additionalContext = readFileSync(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT, "utf8")
  } catch {}
  if (!additionalContext && hookContextRequestId && process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES) {
    const responseFile = join(process.env.CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES, \`\${hookContextRequestId}.txt\`)
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
  // MP-08 / MP-10 / MP-11: an explicit selection must never run with
  // empty hidden instructions when the home/context bridge failed.
  const selectedSkill = hasExplicitSkillMarker(input.prompt ?? "")
  if (!additionalContext && selectedSkill) {
    process.stdout.write(JSON.stringify({ decision: "block",
      reason: "Chariox skill context was not delivered. Refresh Skills and retry this prompt." }))
    process.exit(0)
  }
  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: "UserPromptSubmit",
      additionalContext
    }
  }))
} else if (eventName === "PreToolUse" || eventName === "PermissionRequest") {
  if (input.permission_mode === "bypassPermissions") {
    if (eventName === "PermissionRequest") {
      process.stdout.write(JSON.stringify({
        hookSpecificOutput: {
          hookEventName: "PermissionRequest",
          decision: {
            behavior: "allow"
          }
        }
      }))
    }
  } else {
    const bridgeUrl = process.env.CHARIOX_CLAUDE_NATIVE_HOOK_BRIDGE_URL
    if (bridgeUrl) {
      try {
        const response = await fetch(new URL("/permission", bridgeUrl), {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ ...input, native_origin: nativeOrigin })
        })
        if (response.ok) {
          const decision = await response.json()
          if (decision?.handled && (decision.behavior === "allow" || decision.behavior === "deny")) {
            const hookSpecificOutput = eventName === "PermissionRequest"
              ? {
                  hookEventName: "PermissionRequest",
                  decision: {
                    behavior: decision.behavior,
                    ...(decision.behavior === "deny"
                      ? { message: decision.message ?? "Denied through Chariox." }
                      : {})
                  }
                }
              : {
                  hookEventName: "PreToolUse",
                  permissionDecision: decision.behavior,
                  permissionDecisionReason: decision.message ?? "Resolved through Chariox."
                }
            process.stdout.write(JSON.stringify({ hookSpecificOutput }))
          }
        }
      } catch {}
    }
  }
}
`, "utf8")
}

// Claude runs each hook as a shell command. From source, Node runs the handler;
// the release executable has no Node beside it, so it runs the handler itself.
export function claudeHookShellCommand(
  handlerPath: string,
  release: { version: string | undefined, executable: string } = { version: releaseVersion, executable: process.execPath },
): string {
  return release.version === undefined
    ? `node ${shellQuote(handlerPath)}`
    : `${shellQuote(release.executable)} ${claudeHookCommand} ${shellQuote(handlerPath)}`
}

export function claudeHookSettings(handlerPath: string, command = claudeHookShellCommand(handlerPath)) {
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
