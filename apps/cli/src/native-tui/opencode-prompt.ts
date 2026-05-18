import {
  type PromptAttachmentPart,
  type RuntimeProviderRun,
} from "../cli-types.js"

export type OpenCodePromptBody = {
  parts?: unknown
  text?: unknown
  prompt?: unknown
  model?: unknown
  variant?: unknown
  agent?: unknown
}

export type NativeProviderSelection = {
  model?: string
  variant?: string
  agent?: string
}

export function extractPromptText(body: OpenCodePromptBody): string {
  const parts = Array.isArray(body.parts) ? body.parts : []
  const textParts = parts.flatMap((part) => {
    if (!part || typeof part !== "object") return []
    const record = part as Record<string, unknown>
    return record.type === "text" && typeof record.text === "string" ? [record.text] : []
  })
  if (textParts.length > 0) {
    return textParts.join("\n")
  }
  if (typeof body.text === "string") return body.text
  if (typeof body.prompt === "string") return body.prompt
  return ""
}

export function extractPromptAttachments(body: OpenCodePromptBody): PromptAttachmentPart[] {
  const parts = Array.isArray(body.parts) ? body.parts : []
  return parts.flatMap((part) => {
    if (!part || typeof part !== "object") return []
    const record = part as Record<string, unknown>
    if (record.type !== "file" || typeof record.url !== "string") return []
    return [{
      url: record.url,
      mime: typeof record.mime === "string" ? record.mime : "application/octet-stream",
      filename: typeof record.filename === "string" ? record.filename : null,
    }]
  })
}

export function extractOpenCodeSelection(body: OpenCodePromptBody): NativeProviderSelection | null {
  const selection: NativeProviderSelection = {}
  if (body.model && typeof body.model === "object") {
    const model = body.model as Record<string, unknown>
    const providerId = typeof model.providerID === "string" ? model.providerID : null
    const modelId = typeof model.modelID === "string" ? model.modelID : null
    if (providerId && modelId) {
      selection.model = `${providerId}/${modelId}`
    }
  } else if (typeof body.model === "string") {
    selection.model = body.model
  }
  if (typeof body.variant === "string" && body.variant.trim()) {
    selection.variant = body.variant
  }
  if (typeof body.agent === "string" && body.agent.trim()) {
    selection.agent = body.agent
  }
  return selection.model || selection.variant || selection.agent ? selection : null
}

export function shouldApplyNativeSelection(
  incoming: NativeProviderSelection,
  lastNativeSelection: NativeProviderSelection | null,
  run: RuntimeProviderRun,
) {
  const incomingModel = incoming.model?.trim() || undefined
  const incomingVariant = incoming.variant?.trim() || undefined
  if (!incomingModel && incomingVariant === undefined) {
    return false
  }
  const currentModel = run.model?.trim() || undefined
  const currentVariant = run.variant?.trim() || undefined
  if (incomingModel === currentModel && incomingVariant === currentVariant) {
    return false
  }
  if (
    lastNativeSelection
    && incomingModel === (lastNativeSelection.model?.trim() || undefined)
    && incomingVariant === (lastNativeSelection.variant?.trim() || undefined)
  ) {
    return false
  }
  return true
}

export function compactSelection(selection: NativeProviderSelection): NativeProviderSelection {
  return {
    ...(selection.model ? { model: selection.model } : {}),
    ...(selection.variant ? { variant: selection.variant } : {}),
    ...(selection.agent ? { agent: selection.agent } : {}),
  }
}
