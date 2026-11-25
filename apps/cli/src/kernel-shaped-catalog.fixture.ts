import { fallbackProviderCatalog, type ProviderCatalog } from "./provider-catalog.js"

// What a connected kernel reports: the fallback's providers plus the two Claude backends with the
// kernel's own model list and default. Tests that need Claude models use this, not the fallback.
export const KERNEL_CLAUDE_MODEL = "claude-sonnet-5-5"

export function kernelShapedCatalog(): ProviderCatalog {
  const base = fallbackProviderCatalog()
  const claude = (id: string, name: string) => ({
    id,
    name,
    remote_machine_aliases: [],
    models: {
      [KERNEL_CLAUDE_MODEL]: {
        id: KERNEL_CLAUDE_MODEL,
        name: "Sonnet 5.5",
        status: "active",
        variants: { low: {}, medium: {}, high: {}, xhigh: {}, max: {} },
      },
    },
  })
  return {
    ...base,
    all: [...base.all, claude("claude-headless", "Claude headless"), claude("claude-p", "Claude -p")],
    default: { ...base.default, "claude-headless": KERNEL_CLAUDE_MODEL, "claude-p": KERNEL_CLAUDE_MODEL },
    connected: [...base.connected, "claude-headless", "claude-p"],
  }
}
