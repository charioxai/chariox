import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { mkdtemp, mkdir, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { claudeHookSettings, writeClaudeHookHandler } from "./native-tui/claude-hook-handler.js"

test("Claude yolo PermissionRequest hooks return an explicit allow decision", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-claude-yolo-hook-"))
  const handler = path.join(root, "hook-handler.mjs")
  const events = path.join(root, "events.jsonl")
  const context = path.join(root, "context.txt")
  const contextResponses = path.join(root, "context-responses")
  const permissionResponses = path.join(root, "permission-responses")
  await mkdir(contextResponses)
  await mkdir(permissionResponses)
  await writeClaudeHookHandler(handler)

  try {
    const stdout = execFileSync(process.execPath, [handler], {
      encoding: "utf8",
      input: JSON.stringify({
        hook_event_name: "PermissionRequest",
        permission_mode: "bypassPermissions",
        tool_name: "Bash",
        tool_input: { command: "true" },
      }),
      env: {
        ...process.env,
        CHARIOX_CLAUDE_NATIVE_EVENTS: events,
        CHARIOX_CLAUDE_NATIVE_CONTEXT: context,
        CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES: contextResponses,
        CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES: permissionResponses,
      },
      timeout: 2_000,
    })
    const response = JSON.parse(stdout)
    assert.equal(response.hookSpecificOutput.hookEventName, "PermissionRequest")
    assert.equal(response.hookSpecificOutput.permissionDecision, "allow")
    assert.match(response.hookSpecificOutput.permissionDecisionReason, /yolo permission mode/)
    assert.match(await readFile(events, "utf8"), /"hook_event_name":"PermissionRequest"/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("Claude native settings register the permission hook", () => {
  const settings = claudeHookSettings("/tmp/claude-hook-handler.mjs")
  assert.equal(settings.hooks.PermissionRequest[0]!.matcher, "*")
})
