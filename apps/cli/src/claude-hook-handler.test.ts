import assert from "node:assert/strict"
import { execFileSync, spawn } from "node:child_process"
import { mkdtemp, mkdir, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"

import { claudeHookSettings, writeClaudeHookHandler } from "./native-tui/claude-hook-handler.js"
import { startClaudePermissionBridge } from "./native-tui/claude-permission-bridge.js"

type PermissionContractCase = {
  name: string
  input: Record<string, unknown>
  behavior: string
}

async function runClaudeHookHandler(
  handler: string,
  input: Record<string, unknown>,
  env: NodeJS.ProcessEnv,
): Promise<Record<string, unknown>> {
  const child = spawn(process.execPath, [handler], {
    env,
    stdio: ["pipe", "pipe", "pipe"],
  })
  let stdout = ""
  let stderr = ""
  child.stdout.setEncoding("utf8")
  child.stderr.setEncoding("utf8")
  child.stdout.on("data", (chunk: string) => { stdout += chunk })
  child.stderr.on("data", (chunk: string) => { stderr += chunk })
  child.stdin.end(JSON.stringify(input))
  const exitCode = await new Promise<number | null>((resolve, reject) => {
    child.once("error", reject)
    child.once("exit", resolve)
  })
  assert.equal(exitCode, 0, stderr)
  return JSON.parse(stdout) as Record<string, unknown>
}

test("Claude hook generators obey the shared immediate permission contract", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-claude-yolo-hook-"))
  const handler = path.join(root, "hook-handler.mjs")
  const events = path.join(root, "events.jsonl")
  const context = path.join(root, "context.txt")
  const contextResponses = path.join(root, "context-responses")
  const permissionResponses = path.join(root, "permission-responses")
  await mkdir(contextResponses)
  await mkdir(permissionResponses)
  await writeClaudeHookHandler(handler)
  const contract = JSON.parse(await readFile(fileURLToPath(new URL(
    "../../../fixtures/claude-permission-hook-contract.json",
    import.meta.url,
  )), "utf8")) as PermissionContractCase[]

  try {
    for (const contractCase of contract) {
      const stdout = execFileSync(process.execPath, [handler], {
        encoding: "utf8",
        input: JSON.stringify(contractCase.input),
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
      assert.equal(response.hookSpecificOutput.hookEventName, "PermissionRequest", contractCase.name)
      assert.equal(response.hookSpecificOutput.decision.behavior, contractCase.behavior, contractCase.name)
      assert.equal(response.hookSpecificOutput.permissionDecision, undefined, contractCase.name)
    }
    assert.match(await readFile(events, "utf8"), /"hook_event_name":"PermissionRequest"/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("Claude native settings register the permission hook", () => {
  const settings = claudeHookSettings("/tmp/claude-hook-handler.mjs")
  assert.equal(settings.hooks.PermissionRequest[0]!.matcher, "*")
})

test("Claude permission bridge preserves event-specific allow and deny response shapes", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-claude-permission-bridge-"))
  const handler = path.join(root, "hook-handler.mjs")
  const events = path.join(root, "events.jsonl")
  const context = path.join(root, "context.txt")
  const contextResponses = path.join(root, "context-responses")
  const permissionResponses = path.join(root, "permission-responses")
  await mkdir(contextResponses)
  await mkdir(permissionResponses)
  await writeClaudeHookHandler(handler)
  let interactionCount = 0
  const bridge = await startClaudePermissionBridge({
    client: {
      send: async () => ({
        NativeProviderInteractionResolved: {
          resolution: interactionCount++ === 0
            ? { status: "resolved", choice_id: "deny", reply: null }
            : { status: "resolved", choice_id: "allow_once", reply: "allow" },
        },
      }),
    } as never,
    sessionId: "session-1",
    attachmentId: "attachment-1",
    agentId: "agent-1",
    promptOrigin: { current: "native" },
  })
  const env = {
    ...process.env,
    CHARIOX_CLAUDE_NATIVE_EVENTS: events,
    CHARIOX_CLAUDE_NATIVE_CONTEXT: context,
    CHARIOX_CLAUDE_NATIVE_CONTEXT_RESPONSES: contextResponses,
    CHARIOX_CLAUDE_NATIVE_PERMISSION_RESPONSES: permissionResponses,
    CHARIOX_CLAUDE_NATIVE_HOOK_BRIDGE_URL: bridge.url,
  }

  try {
    const permissionResponse = await runClaudeHookHandler(handler, {
      hook_event_name: "PermissionRequest",
      permission_mode: "default",
      tool_name: "Bash",
      tool_input: { command: "git status --short" },
    }, env)
    assert.deepEqual(permissionResponse, {
      hookSpecificOutput: {
        hookEventName: "PermissionRequest",
        decision: {
          behavior: "deny",
          message: "Denied through Chariox.",
        },
      },
    })

    const preToolResponse = await runClaudeHookHandler(handler, {
      hook_event_name: "PreToolUse",
      permission_mode: "default",
      tool_name: "Write",
      tool_input: { file_path: "/tmp/example" },
    }, env)
    assert.deepEqual(preToolResponse, {
      hookSpecificOutput: {
        hookEventName: "PreToolUse",
        permissionDecision: "allow",
        permissionDecisionReason: "Resolved through Chariox.",
      },
    })
    assert.equal(interactionCount, 2)
  } finally {
    await bridge.stop()
    await rm(root, { recursive: true, force: true })
  }
})
