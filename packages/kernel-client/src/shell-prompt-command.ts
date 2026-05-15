import type {
  AgentInstance,
  PromptQueueItem,
  PromptSubmittedPayload,
  RuntimeSession,
  SessionHistoryPage,
  SessionHistoryPageEntry,
} from "./kernel-types.js"
import {
  getSessionHistoryRequest,
  getSessionStateRequest,
  pumpTerminalOutputRequest,
  submitPromptRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import { formatAgentRef } from "./shell-agent-format.js"
import {
  resolveShellAgent,
  tryResolveShellAgent,
} from "./shell-agent-resolver.js"
import {
  formatPromptBlob,
  formatPromptReply,
  formatPromptSummary,
} from "./shell-history-format.js"
import {
  expectSessionState,
  resolveShellAttachmentId,
} from "./shell-session-attachment.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

type ParsedPromptArgs = {
  agentRef?: string | undefined
  prompt: string
  wait: boolean
  showReply: boolean
  showSummary: boolean
}

export type ShellPromptCommandDeps = {
  client: ShellKernelClient
}

export async function executePromptCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellPromptCommandDeps,
): Promise<ShellCommandResult> {
  if (!context.sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }
  const attachmentId = await resolveShellAttachmentId(context, deps)
  if (!attachmentId.ok) {
    return { ok: false, message: attachmentId.message }
  }
  const promptArgs = await parsePromptArgs(parsed.args, context, deps)
  if (!promptArgs.ok) {
    return { ok: false, message: promptArgs.message }
  }
  const target = promptArgs.agent
  const promptText = promptArgs.options.prompt.endsWith("\n")
    ? promptArgs.options.prompt
    : `${promptArgs.options.prompt}\n`
  const response = await deps.client.send(submitPromptRequest(
    context.sessionId,
    attachmentId.attachmentId,
    target.id,
    promptText,
    [],
  ))
  const payload = expectVariant<PromptSubmittedPayload>(response, "PromptSubmitted")
  const prompt = extractSubmittedPrompt(payload, target.id)
  const promptId = prompt?.id ?? "unknown-prompt"
  const waitForCompletion = promptArgs.options.wait || promptArgs.options.showReply || promptArgs.options.showSummary
  if (!waitForCompletion) {
    return {
      ok: true,
      message: `prompt ${promptId} submitted to ${formatAgentRef(target)}`,
      data: { prompt, session: payload.session },
      contextUpdates: { agentId: target.id },
    }
  }

  const completedSession = await waitForPromptCompletion(context.sessionId, attachmentId.attachmentId, target.id, promptId, deps)
  const history = await readPromptHistory(context.sessionId, target.id, promptText, deps)
  const lines = [`prompt ${promptId} completed`]
  if (promptArgs.options.showReply) {
    lines.push(formatPromptBlob(promptId, "reply", formatPromptReply(history)))
  } else if (promptArgs.options.showSummary) {
    lines.push(formatPromptBlob(promptId, "summary", formatPromptSummary(history)))
  }
  return {
    ok: true,
    message: lines.join("\n"),
    data: { prompt, session: completedSession, history },
    contextUpdates: { agentId: target.id },
  }
}

async function parsePromptArgs(
  args: string[],
  context: ShellContext,
  deps: ShellPromptCommandDeps,
): Promise<
  | { ok: true; agent: AgentInstance; options: ParsedPromptArgs }
  | { ok: false; message: string }
> {
  const positional: string[] = []
  let wait = false
  let showReply = false
  let showSummary = false
  for (const arg of args) {
    const normalized = normalizeShellFlag(arg)
    if (normalized === "--wait") {
      wait = true
    } else if (normalized === "--show-reply") {
      showReply = true
    } else if (normalized === "--show-summary") {
      showSummary = true
    } else {
      positional.push(arg)
    }
  }
  if (showReply && showSummary) {
    return { ok: false, message: "usage: prompt [agent-ref] <prompt> [--wait] [--show-reply|--show-summary]" }
  }
  if (positional.length === 0) {
    return { ok: false, message: "usage: prompt [agent-ref] <prompt> [--wait] [--show-reply|--show-summary]" }
  }

  let agentRef: string | undefined
  let promptParts = positional
  if (positional.length > 1) {
    const explicitAgent = await tryResolveShellAgent(context, deps, positional[0])
    if (explicitAgent.ok) {
      agentRef = positional[0]
      promptParts = positional.slice(1)
      return {
        ok: true,
        agent: explicitAgent.agent,
        options: { agentRef, prompt: promptParts.join(" "), wait, showReply, showSummary },
      }
    }
  }
  const defaultAgent = await resolveShellAgent(context, deps, undefined)
  if (!defaultAgent.ok) {
    return { ok: false, message: defaultAgent.message.replace("usage: mcp|skill grants <agent-ref>", "usage: prompt [agent-ref] <prompt>") }
  }
  return {
    ok: true,
    agent: defaultAgent.agent,
    options: { prompt: promptParts.join(" "), wait, showReply, showSummary },
  }
}

function normalizeShellFlag(value: string): string {
  return value.startsWith("—") ? `--${value.slice(1)}` : value
}

function extractSubmittedPrompt(payload: PromptSubmittedPayload, targetAgentId: string): PromptQueueItem | null {
  const variants = Object.values(payload.outcome ?? {})
  for (const variant of variants) {
    if (variant && typeof variant === "object" && "prompt" in variant) {
      const prompt = (variant as { prompt?: PromptQueueItem | null }).prompt
      if (prompt) return prompt
    }
  }
  const state = payload.session.prompt_states?.[targetAgentId]
  return state?.active_prompt
    ?? state?.queued_prompts?.[state.queued_prompts.length - 1]
    ?? (payload.session.active_prompt?.target_agent_id === targetAgentId ? payload.session.active_prompt : null)
    ?? [...payload.session.queued_prompts].reverse().find((prompt) => prompt.target_agent_id === targetAgentId)
    ?? null
}

async function waitForPromptCompletion(
  sessionId: string,
  attachmentId: string,
  agentId: string,
  promptId: string,
  deps: ShellPromptCommandDeps,
): Promise<RuntimeSession> {
  const deadline = Date.now() + 120_000
  let latest: RuntimeSession | null = null
  while (Date.now() < deadline) {
    await deps.client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => ({}))
    const response = await deps.client.send(getSessionStateRequest(sessionId))
    latest = expectSessionState(response)
    if (!sessionHasPrompt(latest, agentId, promptId)) {
      return latest
    }
    await sleep(250)
  }
  throw new Error(`timed out waiting for prompt ${promptId}`)
}

async function readPromptHistory(
  sessionId: string,
  agentId: string,
  promptText: string,
  deps: ShellPromptCommandDeps,
): Promise<SessionHistoryPageEntry[]> {
  const response = await deps.client.send(getSessionHistoryRequest(sessionId, 12, 120_000, null, agentId))
  const page = expectVariant<SessionHistoryPage>(response, "SessionHistory")
  const entries = [...page.entries].sort((left, right) => left.entry_index - right.entry_index)
  const promptIndex = findPromptHistoryIndex(entries, promptText)
  return promptIndex === null
    ? entries.filter((entry) => entry.entry.kind !== "user_prompt")
    : entries.filter((entry) => entry.entry_index > promptIndex && entry.entry.kind !== "user_prompt")
}

function findPromptHistoryIndex(entries: SessionHistoryPageEntry[], promptText: string): number | null {
  const normalizedPrompt = promptText.trim()
  const matches = entries.filter((entry) => entry.entry.kind === "user_prompt" && entry.entry.text.trim() === normalizedPrompt)
  const matched = matches[matches.length - 1]
  if (matched) return matched.entry_index
  const lastPrompt = [...entries].reverse().find((entry) => entry.entry.kind === "user_prompt")
  return lastPrompt?.entry_index ?? null
}

function sessionHasPrompt(session: RuntimeSession, agentId: string, promptId: string): boolean {
  const state = session.prompt_states?.[agentId]
  return state?.active_prompt?.id === promptId
    || Boolean(state?.queued_prompts?.some((prompt) => prompt.id === promptId))
    || session.active_prompt?.id === promptId
    || session.queued_prompts.some((prompt) => prompt.id === promptId)
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
