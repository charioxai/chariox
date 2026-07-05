import {
  ACTIVE_STATUS_FALLBACK,
  getProviderActivityLabel,
  isProviderIdleStatus,
  normalizeProviderActivityLabel,
  shouldRenderProviderStatus,
  toProviderPresentParticiplePhrase,
} from "@arroba/tool-display"

export {
  ACTIVE_STATUS_FALLBACK,
  getProviderActivityLabel,
  isProviderIdleStatus,
  normalizeProviderActivityLabel,
  shouldRenderProviderStatus,
  toProviderPresentParticiplePhrase,
}

const TOOL_ACTIVITY_LABELS: Record<string, string> = {
  apply_patch: "patching",
  read: "reading",
}

export function getToolActivityLabel(tool?: string | null): string | null {
  const normalized = normalizeProviderActivityLabel(tool)
  if (!normalized) {
    return null
  }
  return TOOL_ACTIVITY_LABELS[normalized] ?? toProviderPresentParticiplePhrase(normalized)
}

export function chooseVisibleActivityLabel(
  providerActivity: string | null,
  activeToolActivity: string | null,
): string | null {
  return activeToolActivity ?? providerActivity
}
