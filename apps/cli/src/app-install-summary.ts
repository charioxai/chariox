import type { AppInstallPrompt } from "@chariox/kernel-client/kernel-types"

const OPERATION_VERB: Record<AppInstallPrompt["operation"], string> = { install: "Install", update: "Update", reinstall: "Reinstall" }
const ASSET_VERB: Record<string, string> = { read: "Read", write: "Change", run: "Run" }

const isRecord = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value)
const isText = (value: unknown): value is string => typeof value === "string" && value.trim().length > 0
const strings = (value: unknown): string[] => Array.isArray(value) ? value.filter(isText) : []

/** The App an install prompt is about, as the lines the owner reads before approving. */
export function appInstallSummary(install: AppInstallPrompt): { readonly action: string; readonly heading: string; readonly detail: string; readonly access: readonly string[] } {
  const { review } = install
  const publisher = isText(review["publisherName"]) ? review["publisherName"] : isText(review["publisherId"]) ? review["publisherId"] : null
  const capabilities = isRecord(review["capabilities"]) ? review["capabilities"] : {}
  const access: string[] = []
  for (const destination of Array.isArray(capabilities["network"]) ? capabilities["network"] : []) {
    if (isRecord(destination) && isText(destination["origin"])) access.push(`Connect to ${destination["origin"]} (${strings(destination["methods"]).join(", ")})`)
  }
  for (const mode of strings(capabilities["clipboard"])) access.push(mode === "write" ? "Write to your clipboard" : "Read your clipboard")
  if (strings(capabilities["externalFiles"]).length > 0) access.push("Open files you choose")
  for (const kind of ["workflows", "agents"] as const) {
    const modes = strings(capabilities[kind]).map((mode) => ASSET_VERB[mode] ?? mode)
    if (modes.length > 0) access.push(`${modes.join(", ")} ${kind}`)
  }
  for (const connection of Array.isArray(capabilities["connections"]) ? capabilities["connections"] : []) {
    if (isRecord(connection) && isText(connection["generator"])) access.push(`Use ${connection["generator"]}: ${strings(connection["actions"]).join(", ")}`)
  }
  return {
    action: OPERATION_VERB[install.operation],
    heading: `${OPERATION_VERB[install.operation]} ${String(review["appId"])}`,
    detail: [isText(review["version"]) ? `Version ${review["version"]}` : null, publisher ? `Published by ${publisher}` : null].filter(Boolean).join(" · "),
    access: access.length > 0 ? access : ["No access beyond its own data"],
  }
}
