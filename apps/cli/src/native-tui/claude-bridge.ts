import { writeFile } from "node:fs/promises"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

import {
  normalizeRuntimeSessionWithAgentActivity,
  type PromptQueueItem,
  type RuntimeSession,
} from "../cli-types.js"
import {
  sessionActivePromptForAgent,
} from "@arroba/kernel-client/session-prompt-identity"
import {
  promptSubmittedPromptIdFromResponse,
} from "@arroba/kernel-client/prompt-submission"
import { LocalIpcClient } from "../ipc.js"
import {
  appendNativeProviderOutputRequest,
  completePromptRequest,
  getSessionStateRequest,
  submitPromptRequest,
} from "../ipc-requests.js"
import { preparePromptAttachmentsForSubmit } from "../prompt-attachment-transfer.js"
import {
  extractClaudeNativePromptAttachments,
  formatClaudeAttachmentContext,
  formatClaudeNativeAttachmentPromptSuffix,
  joinClaudeAdditionalContext,
  joinClaudeVisiblePrompt,
} from "./claude-attachments.js"
import type { ClaudePromptOriginState } from "./claude-permission-bridge.js"
import {
  buildClaudeNativeSkillContext,
  writeClaudeHookContextResponse,
} from "./claude-skill-context.js"
import {
  readClaudeHookEvents,
  waitForAssistantText,
} from "./claude-transcript.js"
import { hiddenInstructionsEnd, hiddenInstructionsStart, redactHiddenInstructions } from "./hidden-instructions.js"

type ClaudeBridgeDebug = (label: string, payload: unknown) => void

type ClaudeBridgeOptions = {
  client: LocalIpcClient
  sessionId: string
  attachmentId: string
  agentId: string
  providerRunId: string
  eventsFile: string
  contextFile: string
  attachmentContextDir: string
  hookContextResponseDir: string
  workspace: string
  worktree: string
  inlineLocalAttachments: boolean
  promptOrigin: ClaudePromptOriginState
  submitPrompt: (prompt: string) => Promise<void>
  debug: ClaudeBridgeDebug
}

export function startClaudeBridge(options: ClaudeBridgeOptions): { stop: () => void } {
  let stopped = false
  let nextEventIndex = 0
  let activePromptId: string | null = null
  const injectedPromptIds = new Set<string>()
  const nativeSubmittedPromptIds = new Set<string>()
  const transcriptLineOffsets = new Map<string, number>()

  const loop = async () => {
    while (!stopped) {
      try {
        const events = await readClaudeHookEvents(options.eventsFile)
        for (const event of events.slice(nextEventIndex)) {
          nextEventIndex = Math.max(nextEventIndex, event.index + 1)
          if (event.hook_event_name === "UserPromptSubmit" && event.prompt) {
            const prompt = event.prompt.trim()
            const isInjected = activePromptId && injectedPromptIds.has(activePromptId)
            if (!isInjected) {
              const attachments = await preparePromptAttachmentsForSubmit(
                extractClaudeNativePromptAttachments(prompt, options.worktree),
                { inlineLocalFiles: options.inlineLocalAttachments },
              )
              if (attachments.length > 0) {
                options.debug("native_prompt_attachments_observed", {
                  attachmentCount: attachments.length,
                })
              }
              if (event.hook_context_request_id) {
                const context = await buildClaudeNativeSkillContext(
                  options.client,
                  options.sessionId,
                  options.workspace,
                  options.agentId,
                  prompt,
                )
                await writeClaudeHookContextResponse(
                  options.hookContextResponseDir,
                  event.hook_context_request_id,
                  context,
                )
              }
              const response = await options.client.send<Record<string, unknown>>(
                submitPromptRequest(options.sessionId, options.attachmentId, options.agentId, prompt, attachments),
              )
              const submittedPrompt = promptSubmittedPromptIdFromResponse(response, options.agentId)
              if (submittedPrompt) {
                activePromptId = submittedPrompt
                nativeSubmittedPromptIds.add(submittedPrompt)
                options.promptOrigin.current = "native"
              } else {
                const state = await sessionState(options.client, options.sessionId)
                activePromptId = promptForAgent(state, options.agentId)?.id ?? activePromptId
                if (activePromptId) {
                  nativeSubmittedPromptIds.add(activePromptId)
                  options.promptOrigin.current = "native"
                }
              }
            }
          } else if (event.hook_event_name === "Stop") {
            const output = event.transcript_path
              ? await waitForAssistantText(event.transcript_path, transcriptLineOffsets)
              : ""
            if (output.trim()) {
              await options.client.send<Record<string, unknown>>(
                appendNativeProviderOutputRequest(
                  options.sessionId,
                  options.attachmentId,
                  options.providerRunId,
                  "provider_output",
                  output.endsWith("\n") ? output : `${output}\n`,
                  `claude-native-${Date.now()}`,
                ),
              )
            }
            await options.client.send<Record<string, unknown>>(completePromptRequest(options.sessionId))
              .catch(() => ({}))
            activePromptId = null
            options.promptOrigin.current = null
            await writeFile(options.contextFile, "", "utf8").catch(() => {})
          }
        }

        const state = await sessionState(options.client, options.sessionId)
        const activePrompt = promptForAgent(state, options.agentId)
        if (activePrompt && activePrompt.id !== activePromptId && !nativeSubmittedPromptIds.has(activePrompt.id)) {
          activePromptId = activePrompt.id
          injectedPromptIds.add(activePrompt.id)
          options.promptOrigin.current = "external"
          const hidden = extractHiddenInstructions(activePrompt.prompt)
          const attachmentContext = await formatClaudeAttachmentContext(
            activePrompt.attachments ?? [],
            options.attachmentContextDir,
          )
          const nativeAttachmentSuffix = await formatClaudeNativeAttachmentPromptSuffix(
            activePrompt.attachments ?? [],
            options.attachmentContextDir,
          )
          await writeFile(options.contextFile, joinClaudeAdditionalContext(hidden, attachmentContext), "utf8")
          if (hidden.trim()) {
            options.debug("hidden_instructions_forwarded", {
              promptId: activePrompt.id,
            })
          }
          if ((activePrompt.attachments?.length ?? 0) > 0) {
            options.debug("attachments_forwarded", {
              promptId: activePrompt.id,
              attachmentCount: activePrompt.attachments?.length ?? 0,
            })
          }
          const visible = redactHiddenInstructions(activePrompt.prompt).trim()
          const prompt = joinClaudeVisiblePrompt(nativeAttachmentSuffix, visible)
          if (prompt) {
            await options.submitPrompt(prompt)
          }
        }
      } catch (error) {
        process.stderr.write(`[arroba claude native-tui] bridge warning: ${formatError(error)}\n`)
      }
      await sleep(500)
    }
  }
  void loop()
  return {
    stop: () => {
      stopped = true
    },
  }
}

function extractHiddenInstructions(prompt: string): string {
  const start = prompt.indexOf(hiddenInstructionsStart)
  if (start < 0) return ""
  const end = prompt.indexOf(hiddenInstructionsEnd, start)
  if (end < 0) return prompt.slice(start)
  return prompt.slice(start + hiddenInstructionsStart.length, end).trim()
}

export function promptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  return sessionActivePromptForAgent(
    session as Parameters<typeof sessionActivePromptForAgent>[0],
    agentId,
  ) as PromptQueueItem | null
}

async function sessionState(client: LocalIpcClient, sessionId: string): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(getSessionStateRequest(sessionId))
  return normalizeRuntimeSessionWithAgentActivity(
    expectVariant<{
      session: RuntimeSession
      agent_activity?: RuntimeSession["agent_activity"] | null
      agent_activity_revision?: number | null
    }>(response, "SessionState"),
  )
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`expected ${variant} response, received ${JSON.stringify(response)}`)
  }
  return response[variant] as T
}

function formatError(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
