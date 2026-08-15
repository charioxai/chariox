import {
  listPromptSettingsRequest,
  resetAllPromptSettingsRequest,
  resetPromptSettingRequest,
} from "./ipc-prompt-settings-requests.js"
import type { ShellCommandResult } from "./shell-core.js"

type PromptSetting = {
  id: string
  title: string
  current_sha256: string
  revision: number
  editable: boolean
  protected: boolean
  source: string
}

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

function listed(response: Record<string, unknown>): PromptSetting[] {
  const payload = response.PromptSettingsListed
  if (!payload || typeof payload !== "object") throw new Error("kernel did not return prompt settings")
  const settings = (payload as { settings?: unknown }).settings
  if (!Array.isArray(settings)) throw new Error("kernel returned an invalid prompt settings catalog")
  return settings as PromptSetting[]
}

function resetResult(response: Record<string, unknown>): PromptSetting[] {
  if (response.PromptSetting && typeof response.PromptSetting === "object") {
    const setting = (response.PromptSetting as { setting?: PromptSetting }).setting
    return setting ? [setting] : []
  }
  if (response.PromptSettingsReset && typeof response.PromptSettingsReset === "object") {
    const settings = (response.PromptSettingsReset as { settings?: unknown }).settings
    return Array.isArray(settings) ? settings as PromptSetting[] : []
  }
  throw new Error("kernel did not return a reset result")
}

export async function executePromptSettingsCommand(
  args: string[],
  client: ShellKernelClient,
): Promise<ShellCommandResult> {
  const [action = "list", ...rest] = args
  const id = action === "reset" ? rest[0] : undefined
  const flags = action === "reset" ? rest.slice(1) : rest
  const confirmed = flags.includes("--confirm")
  if (action === "list") {
    const settings = listed(await client.send(listPromptSettingsRequest()))
    return {
      ok: true,
      message: settings.map((setting) =>
        `${setting.id}  ${setting.source}${setting.editable ? "" : "  read-only"}${setting.protected ? "  protected" : ""}`,
      ).join("\n") || "no prompt settings",
      data: settings,
    }
  }
  if (action !== "reset" && action !== "reset-all") {
    return { ok: false, message: "usage: settings prompts list|reset <id> [--confirm]|reset-all [--confirm]" }
  }
  if (!confirmed) {
    return { ok: false, message: `reset requires --confirm (settings prompts ${action}${id ? ` ${id}` : ""} --confirm)` }
  }
  const settings = listed(await client.send(listPromptSettingsRequest()))
  if (action === "reset") {
    if (!id) return { ok: false, message: "usage: settings prompts reset <id> --confirm" }
    const setting = settings.find((candidate) => candidate.id === id)
    if (!setting) return { ok: false, message: `unknown prompt setting: ${id}` }
    if (!setting.editable && !setting.protected) return { ok: false, message: `prompt setting is not resettable: ${id}` }
    const result = resetResult(await client.send(resetPromptSettingRequest(id, setting.revision, setting.current_sha256)))
    return { ok: true, message: `reset ${result[0]?.id ?? id} to the bundled default`, data: result[0] }
  }
  const expected = Object.fromEntries(settings.map((setting) => [setting.id, {
    revision: setting.revision,
    sha256: setting.current_sha256,
  }]))
  const result = resetResult(await client.send(resetAllPromptSettingsRequest(expected)))
  return { ok: true, message: `reset ${result.length} prompt settings to bundled defaults`, data: result }
}
