import { renderPatchFilesDiffHtml } from "../markdown/diff-render.js"
import {
  renderTranscriptHtml as renderTranscriptHtmlDefault,
} from "../transcript-renderer.js"
import { isToolDisplayIcon, renderToolIcon } from "./tool-icons.js"
import {
  formatToolDisplay,
  type ToolDisplay,
  type ToolDisplayBlock,
} from "@chariox/tool-display"
import type {
  TerminalPromptAttachment,
  TerminalPromptAttachmentView,
} from "./prompt-attachment-state.js"
import {
  terminalOutputEntryShouldRenderInAgentPane,
} from "./terminal-output-render-policy.js"
import type {
  FreeformTranscriptRole,
  TerminalOutputEntry,
  TerminalOutputRole,
} from "./runtime-state.js"
import {
  terminalActivitySummary,
} from "./activity-group-projection.js"

export {
  terminalActivitySummary,
} from "./activity-group-projection.js"

type EscapeHtml = (value: string) => string
type RenderTranscriptHtml = (text: string) => string

export type FreeformTranscriptRenderOptions = {
  readonly expandedEntryKeys?: ReadonlySet<string>
  /** Whether a row is its active turn's live tail ("Thinking…"). */
  readonly isLiveEntry?: (entry: TerminalOutputEntry) => boolean
  readonly pendingAttachments?: readonly TerminalPromptAttachment[]
  readonly renderTranscriptHtml?: RenderTranscriptHtml
  readonly escapeHtml?: EscapeHtml
}

type FreeformTranscriptRenderContext = {
  readonly expandedEntryKeys: ReadonlySet<string>
  readonly isLiveEntry: (entry: TerminalOutputEntry) => boolean
  readonly pendingAttachments: readonly TerminalPromptAttachment[]
  readonly renderTranscriptHtml: RenderTranscriptHtml
  readonly escapeHtml: EscapeHtml
}

const emptyExpandedEntryKeys = new Set<string>()
const emptyPendingAttachments: readonly TerminalPromptAttachment[] = []

type AgentMessageDisplay = {
  readonly alias: string
  readonly message: string
}

export function renderFreeformMessage(
  role: FreeformTranscriptRole,
  text: string,
  attachments: readonly TerminalPromptAttachmentView[] = [],
  renderKey?: string,
  options: FreeformTranscriptRenderOptions = {},
  promptOrigin?: string | null,
): string {
  const context = freeformTranscriptRenderContext(options)
  const renderKeyAttribute = renderKey ? ` data-render-key="${context.escapeHtml(renderKey)}"` : ""
  if (role === "user") {
    const agentMessage = agentMessageDisplay(text)
    const promptText = (agentMessage?.message ?? text).trim()
    const triggerLabel = triggerPromptOriginLabel(promptOrigin)
    const header = agentMessage
      ? `<header class="freeform-agent-message-header">agent <strong>${context.escapeHtml(agentMessage.alias)}</strong> message:</header>`
      : triggerLabel
        ? `<header class="freeform-agent-message-header" data-trigger-origin="${context.escapeHtml(promptOrigin ?? "")}"><strong>${context.escapeHtml(triggerLabel)}</strong></header>`
        : ""
    const attachmentStrip = attachments.length
      ? renderPromptAttachmentStrip(attachments, "sent", options)
      : ""
    const body = `${header}${attachmentStrip}${promptText ? `<p>${context.escapeHtml(promptText)}</p>` : ""}`
    const agentMessageClass = agentMessage ? " freeform-agent-message-prompt" : ""
    return `<section class="freeform-message freeform-user-prompt${agentMessageClass}"${renderKeyAttribute}>${body}</section>`
  }
  if (role === "tool") {
    return renderFreeformToolMessage(text, options)
  }
  const className = freeformOutputClassName(role)
  return `<section class="freeform-message ${className}"${renderKeyAttribute}>${context.renderTranscriptHtml(text)}</section>`
}

/** What fired a prompt that no person typed; the kernel records it as the prompt's origin. */
export function triggerPromptOriginLabel(origin: string | null | undefined): string | null {
  switch (origin?.trim().toLowerCase()) {
    case "schedule": return "schedule trigger"
    case "http": return "HTTP trigger"
    case "notification": return "notification"
    case "app_event": return "App event"
    default: return null
  }
}

export function agentMessageDisplay(text: string): AgentMessageDisplay | null {
  const canonical = text.match(/^agent\s+([^\r\n:]+?)\s+message:\s*(?:\r?\n\r?\n)?([\s\S]*)$/i)
  if (canonical) {
    return {
      alias: canonical[1]?.trim() ?? "",
      message: canonical[2] ?? "",
    }
  }
  const legacy = text.match(/^Message from @?([^\r\n:]+):\s*(?:\r?\n\r?\n)?([\s\S]*)$/i)
  if (!legacy) {
    return null
  }
  return {
    alias: legacy[1]?.trim() ?? "",
    message: legacy[2] ?? "",
  }
}

export function renderFreeformTranscriptEntry(
  entry: TerminalOutputEntry,
  options: FreeformTranscriptRenderOptions = {},
): string {
  if (!terminalOutputEntryShouldRenderInAgentPane(entry)) {
    return ""
  }
  const role = entry.role === "assistant" ? "agent" : entry.role
  const text = entry.sourceText ?? entry.text
  const renderKey = `entry:${entry.key}`
  if (!isCollapsibleTerminalEntry(entry)) {
    return renderFreeformMessage(role, text, entry.attachments ?? [], renderKey, options, entry.promptOrigin)
  }

  return renderCollapsibleFreeformMessage(entry, text, renderKey, options)
}

export function renderPromptAttachmentStrip(
  attachments: readonly TerminalPromptAttachmentView[],
  context: "pending" | "sent",
  options: FreeformTranscriptRenderOptions = {},
): string {
  return `
    <div class="freeform-attachment-strip" data-context="${context}">
      ${attachments.map((attachment) => renderPromptAttachmentChip(attachment, context, options)).join("")}
    </div>
  `
}

export function renderPromptAttachmentChip(
  attachment: TerminalPromptAttachmentView,
  attachmentContext: "pending" | "sent",
  options: FreeformTranscriptRenderOptions = {},
): string {
  const context = freeformTranscriptRenderContext(options)
  const pending = attachmentContext === "pending"
    ? context.pendingAttachments.find((candidate) => candidate.id === attachment.id)
    : null
  const progress = pending?.status === "loading"
    ? `<span class="freeform-attachment-progress">${Math.max(1, Math.round(pending.progress * 100))}%</span>`
    : ""
  const error = pending?.status === "error" ? pending.error || "attachment failed" : null
  const title = error ? ` title="${context.escapeHtml(error)}"` : ""
  const meta = error ?? formatAttachmentSize(attachment.size)
  const remove = attachmentContext === "pending"
    ? `<button class="freeform-attachment-remove" type="button" data-terminal-attachment-remove="${context.escapeHtml(attachment.id)}" aria-label="Remove ${context.escapeHtml(attachment.filename)}">x</button>`
    : ""
  const media = attachment.kind === "image" && attachment.previewUrl
    ? `<img src="${context.escapeHtml(attachment.previewUrl)}" alt="">`
    : `<span class="freeform-attachment-file">${context.escapeHtml(attachmentFileLabel(attachment))}</span>`
  return `
    <span class="freeform-attachment-chip" data-kind="${context.escapeHtml(attachment.kind)}" data-status="${context.escapeHtml(pending?.status ?? "ready")}"${title}>
      <span class="freeform-attachment-thumb">${media}${progress}</span>
      <span class="freeform-attachment-meta">
        <span>${context.escapeHtml(attachment.filename)}</span>
        ${attachment.kind === "note" && !error ? "" : `<em>${context.escapeHtml(meta)}</em>`}
      </span>
      ${remove}
    </span>
  `
}

export function attachmentFileLabel(attachment: TerminalPromptAttachmentView): string {
  if (attachment.kind === "pdf") {
    return "PDF"
  }
  if (attachment.kind === "text") {
    return "TXT"
  }
  if (attachment.kind === "note") {
    return "NOTE"
  }
  const extension = attachment.filename.includes(".")
    ? attachment.filename.split(".").pop()?.slice(0, 4).toUpperCase()
    : ""
  return extension || "FILE"
}

export function formatAttachmentSize(size: number): string {
  if (size < 1024) {
    return `${size} B`
  }
  if (size < 1024 * 1024) {
    return `${(size / 1024).toFixed(size < 10 * 1024 ? 1 : 0)} KB`
  }
  return `${(size / (1024 * 1024)).toFixed(size < 10 * 1024 * 1024 ? 1 : 0)} MB`
}

export function isCollapsibleTerminalEntry(entry: TerminalOutputEntry): boolean {
  if (entry.role === "user") {
    return false
  }
  // Hydrated assistant output is ordinary transcript content. An assistant
  // blob still attached to an entry is only an unloaded or failed placeholder,
  // so keep its disclosure available for loading and retry.
  if (entry.role === "assistant") {
    return Boolean(entry.historyBlob)
  }
  if (entry.historyBlob) {
    return true
  }
  if (!terminalOutputEntryShouldRenderInAgentPane(entry)) {
    return false
  }
  return entry.role !== "error"
}

function renderCollapsibleFreeformMessage(
  entry: TerminalOutputEntry,
  text: string,
  renderKey: string,
  options: FreeformTranscriptRenderOptions,
): string {
  const context = freeformTranscriptRenderContext(options)
  const expanded = context.expandedEntryKeys.has(entry.key)
  if (entry.historyBlob) {
    return renderHistoryBlobPlaceholder(entry, expanded, renderKey, options)
  }
  if (entry.role === "tool") {
    return renderCollapsibleFreeformToolMessage(entry.key, text, expanded, renderKey, options)
  }
  const className = freeformOutputClassName(entry.role)
  const live = entry.role === "reasoning" && context.isLiveEntry(entry)
  const title = entry.role === "reasoning" ? (live ? "Thinking…" : "Thought") : terminalActivityTitle(entry.role)
  // A live thought previews its latest line, a settled one its first.
  const thoughtLine = live ? text.split("\n").map((line) => line.trim()).filter(Boolean).at(-1) ?? "" : text
  const action = entry.role === "reasoning" ? terminalActivitySummary(thoughtLine).replace(/\*\*([^*]+)\*\*/g, "$1") : terminalActivitySummary(text)
  return `
    <section class="freeform-message ${className} freeform-collapsible-output" data-render-key="${context.escapeHtml(renderKey)}" data-entry-key="${context.escapeHtml(entry.key)}" data-expanded="${expanded ? "true" : "false"}">
      ${renderFreeformBlobHeader(title, action, expanded, options, { running: live })}
      <div class="freeform-blob-body" ${expanded ? "" : "hidden"}>
        ${expanded ? context.renderTranscriptHtml(text) : ""}
      </div>
    </section>
  `
}

function renderHistoryBlobPlaceholder(
  entry: TerminalOutputEntry,
  expanded: boolean,
  renderKey: string,
  options: FreeformTranscriptRenderOptions,
): string {
  const context = freeformTranscriptRenderContext(options)
  const blob = entry.historyBlob
  if (!blob) {
    return ""
  }
  const body = blob.loading
    ? "loading..."
    : blob.error
      ? `failed: ${blob.error}`
      : blob.loaded
        ? entry.text
        : ""
  const className = freeformOutputClassName(entry.role)
  const bodyClassName = entry.role === "tool"
    ? "freeform-blob-body freeform-tool-body"
    : "freeform-blob-body"
  return `
    <section class="freeform-message ${className} freeform-collapsible-output" data-render-key="${context.escapeHtml(renderKey)}" data-entry-key="${context.escapeHtml(entry.key)}" data-expanded="${expanded ? "true" : "false"}">
      ${renderFreeformBlobHeader(blob.kind === "provider_reasoning" && !blob.title.startsWith("Thought") ? "Thought" : blob.title, blob.summary, expanded, options, { icon: blob.icon })}
      <div class="${bodyClassName}" ${expanded ? "" : "hidden"}>
        ${expanded && body ? context.renderTranscriptHtml(body) : ""}
      </div>
    </section>
  `
}

function freeformOutputClassName(role: FreeformTranscriptRole | TerminalOutputRole): string {
  if (role === "reasoning") {
    return "freeform-agent-output freeform-reasoning-output"
  }
  if (role === "tool") {
    return "freeform-agent-output freeform-tool-output"
  }
  if (role === "error") {
    return "freeform-agent-output freeform-error-output"
  }
  if (role === "status") {
    return "freeform-agent-output freeform-status-output"
  }
  return "freeform-agent-output"
}

function renderCollapsibleFreeformToolMessage(
  key: string,
  text: string,
  expanded: boolean,
  renderKey: string,
  options: FreeformTranscriptRenderOptions,
): string {
  const context = freeformTranscriptRenderContext(options)
  const display = parseToolDisplay(text)
  if (!display) {
    return `
      <section class="freeform-message freeform-agent-output freeform-tool-output freeform-collapsible-output" data-render-key="${context.escapeHtml(renderKey)}" data-entry-key="${context.escapeHtml(key)}" data-expanded="${expanded ? "true" : "false"}">
        ${renderFreeformBlobHeader("tool", terminalActivitySummary(text), expanded, options)}
        <div class="freeform-blob-body freeform-tool-body" ${expanded ? "" : "hidden"}>
          ${expanded ? context.renderTranscriptHtml(text) : ""}
        </div>
      </section>
    `
  }
  return `
    <section class="freeform-message freeform-agent-output freeform-tool-output freeform-collapsible-output" data-render-key="${context.escapeHtml(renderKey)}" data-tool="${context.escapeHtml(display.tool)}" data-entry-key="${context.escapeHtml(key)}" data-expanded="${expanded ? "true" : "false"}">
      ${renderFreeformBlobHeader(display.title, [display.status?.toUpperCase(), display.summary].filter(Boolean).join(" "), expanded, options, { icon: display.icon, running: display.status === "running" })}
      <div class="freeform-blob-body freeform-tool-body" ${expanded ? "" : "hidden"}>
        ${expanded ? renderCollapsibleToolDisplayBlocks(display.blocks, options) : ""}
      </div>
    </section>
  `
}

export function renderFreeformBlobHeader(
  title: string,
  action: string,
  expanded: boolean,
  options: FreeformTranscriptRenderOptions = {},
  row: { readonly icon?: unknown; readonly running?: boolean } = {},
): string {
  const context = freeformTranscriptRenderContext(options)
  const icon = isToolDisplayIcon(row.icon) ? row.icon : null
  return `
    <button class="freeform-blob-header" type="button" data-freeform-blob-toggle aria-expanded="${expanded ? "true" : "false"}"${icon ? ` data-tool-icon="${icon}"` : ""}>
      ${icon ? `<span class="freeform-blob-icon" aria-hidden="true">${renderToolIcon(icon)}</span>` : ""}
      <span class="freeform-blob-title"${row.running ? ` data-running="true"` : ""}>${context.escapeHtml(title)}</span>
      <span class="freeform-blob-action">${context.escapeHtml(action)}</span>
      <span class="freeform-blob-chevron" aria-hidden="true">${renderFreeformChevronIcon()}</span>
    </button>
  `
}

export function renderFreeformChevronIcon(): string {
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round" focusable="false"><path d="M9 6l6 6-6 6" /></svg>`
}

export function terminalActivityTitle(role: TerminalOutputRole): string {
  if (role === "reasoning") {
    return "thinking"
  }
  if (role === "error") {
    return "error"
  }
  return "status"
}

export function renderFreeformToolMessage(
  text: string,
  options: FreeformTranscriptRenderOptions = {},
): string {
  const context = freeformTranscriptRenderContext(options)
  const display = parseToolDisplay(text)
  if (!display) {
    return `<section class="freeform-message freeform-agent-output freeform-tool-output">${context.renderTranscriptHtml(text)}</section>`
  }
  return `
    <section class="freeform-message freeform-agent-output freeform-tool-output" data-tool="${context.escapeHtml(display.tool)}">
      <header>
        <span class="freeform-tool-chip">${isToolDisplayIcon(display.icon) ? `<span class="freeform-blob-icon" aria-hidden="true">${renderToolIcon(display.icon)}</span>` : ""}${context.escapeHtml(display.title)}</span>
        ${display.status ? `<span class="freeform-tool-status">${context.escapeHtml(display.status.toUpperCase())}</span>` : ""}
        <span>${context.escapeHtml(display.summary)}</span>
      </header>
      <div class="freeform-tool-body">
        ${display.blocks.map((block: ToolDisplayBlock) => renderToolDisplayBlock(block, options)).join("")}
      </div>
    </section>
  `
}

export function parseToolDisplay(text: string): ToolDisplay | null {
  try {
    const parsed = JSON.parse(text) as Record<string, unknown>
    // Observed provider copies can carry `"id": null`; the tool name still titles them.
    if (!parsed || (typeof parsed.id !== "string" && typeof parsed.tool !== "string")) {
      return null
    }
    return formatToolDisplay({ ...parsed, id: String(parsed.id ?? "") } as Parameters<typeof formatToolDisplay>[0])
  } catch {
    return null
  }
}

export function renderToolDisplayBlock(
  block: ToolDisplayBlock,
  options: FreeformTranscriptRenderOptions = {},
): string {
  const context = freeformTranscriptRenderContext(options)
  if (block.kind === "text") {
    return context.renderTranscriptHtml(block.text)
  }
  if (block.kind === "code") {
    return `<figure class="freeform-tool-code"><figcaption>${context.escapeHtml(block.language || "text")}</figcaption><pre><code>${context.escapeHtml(block.text)}</code></pre></figure>`
  }
  if (block.kind === "json") {
    return `<figure class="freeform-tool-code"><figcaption>json</figcaption><pre><code>${context.escapeHtml(JSON.stringify(block.value, null, 2))}</code></pre></figure>`
  }
  if (block.kind === "table") {
    const columns = block.columns as readonly string[]
    const rows = block.rows as readonly (readonly string[])[]
    return `
      <table class="freeform-tool-table">
        <thead><tr>${columns.map((column: string) => `<th>${context.escapeHtml(column)}</th>`).join("")}</tr></thead>
        <tbody>${rows.map((row: readonly string[]) => `<tr>${row.map((cell: string) => `<td>${context.escapeHtml(cell)}</td>`).join("")}</tr>`).join("")}</tbody>
      </table>
    `
  }
  if (block.kind === "patch") {
    return `<div class="freeform-tool-patch">${renderPatchFilesDiffHtml(block.files)}</div>`
  }
  return ""
}

export function renderCollapsibleToolDisplayBlocks(
  blocks: readonly ToolDisplayBlock[],
  options: FreeformTranscriptRenderOptions = {},
): string {
  const combined = combinedShellToolBlocks(blocks, options)
  if (combined) {
    return combined
  }
  return blocks.map((block) => renderToolDisplayBlock(block, options)).join("")
}

export function combinedShellToolBlocks(
  blocks: readonly ToolDisplayBlock[],
  options: FreeformTranscriptRenderOptions = {},
): string | null {
  if (blocks.length !== 2) {
    return null
  }
  const command = blocks[0]
  const output = blocks[1]
  if (command.kind !== "code" || output.kind !== "code") {
    return null
  }
  if (!isShellToolLanguage(command.language) || output.language !== "text") {
    return null
  }
  const context = freeformTranscriptRenderContext(options)
  return `<figure class="freeform-tool-code freeform-tool-code-combined"><pre><code>${context.escapeHtml([command.text.trimEnd(), output.text.trimEnd()].filter(Boolean).join("\n\n"))}</code></pre></figure>`
}

export function isShellToolLanguage(language: string): boolean {
  return ["bash", "sh", "shellscript", "zsh"].includes(language)
}

function freeformTranscriptRenderContext(options: FreeformTranscriptRenderOptions): FreeformTranscriptRenderContext {
  return {
    expandedEntryKeys: options.expandedEntryKeys ?? emptyExpandedEntryKeys,
    isLiveEntry: options.isLiveEntry ?? (() => false),
    pendingAttachments: options.pendingAttachments ?? emptyPendingAttachments,
    renderTranscriptHtml: options.renderTranscriptHtml ?? renderTranscriptHtmlDefault,
    escapeHtml: options.escapeHtml ?? escapeHtmlDefault,
  }
}

function escapeHtmlDefault(value: string): string {
  return value.replace(/[&<>"']/g, (match) => {
    const escapes: Record<string, string> = {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      "\"": "&quot;",
      "'": "&#39;",
    }
    return escapes[match] ?? match
  })
}
