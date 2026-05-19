import { mkdir, readFile, writeFile } from "node:fs/promises"
import path from "node:path"

import { normalizeRuntimeSession, type RuntimeSession } from "../cli-types.js"
import { LocalIpcClient } from "../ipc.js"
import { getSessionStateRequest, getSkillRequest } from "../ipc-requests.js"

export async function writeClaudeHookContextResponse(dir: string, requestId: string, context: string): Promise<void> {
  if (!requestId.trim()) return
  await mkdir(dir, { recursive: true })
  await writeFile(path.join(dir, `${requestId}.txt`), context, "utf8")
}

export async function buildClaudeNativeSkillContext(
  client: LocalIpcClient,
  sessionId: string,
  workspace: string,
  agentId: string,
  prompt: string,
): Promise<string> {
  const session = await sessionState(client, sessionId)
  const agent = session.agents.find((candidate) => candidate.id === agentId)
  const grants = (agent?.extension_grants ?? [])
    .filter((grant) => grant.kind === "skill")
    .map((grant) => grant.name)
  if (grants.length === 0) return ""
  const lines = [
    "Available Arroba skills for this agent:",
    "Use these granted skills as routing hints when they match the task. If a skill is explicitly selected, mentioned, or requested below, follow its full instructions.",
  ]
  const requestedBodies: Array<{ name: string; body: string }> = []
  for (const grant of grants) {
    const response = await client.send<Record<string, unknown>>(getSkillRequest(workspace, grant))
    const skill = expectVariant<{ skill: { name: string; description: string; short_description?: string | null; path: string } }>(response, "Skill").skill
    lines.push(`- \`${skill.name}\`: ${skill.short_description || skill.description}`)
    if (promptExplicitlyRequestsSkill(prompt, skill.name)) {
      const body = await readFile(skill.path, "utf8")
      requestedBodies.push({ name: skill.name, body })
    }
  }
  if (requestedBodies.length > 0) {
    lines.push("", "Full instructions for explicitly requested Arroba skills:")
    for (const { name, body } of requestedBodies) {
      lines.push(`<arroba_skill name="${name}">`, body.trim(), "</arroba_skill>")
    }
  }
  return lines.join("\n")
}

async function sessionState(client: LocalIpcClient, sessionId: string): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(getSessionStateRequest(sessionId))
  return normalizeRuntimeSession(expectVariant<{ session: RuntimeSession }>(response, "SessionState").session)
}

function promptExplicitlyRequestsSkill(prompt: string, skillName: string): boolean {
  const normalizedPrompt = prompt.toLowerCase()
  const normalizedSkill = skillName.toLowerCase()
  const explicitMarkers = [
    `@${normalizedSkill}`,
    `\`${normalizedSkill}\``,
    `/skill ${normalizedSkill}`,
    `skill ${normalizedSkill}`,
    `use ${normalizedSkill}`,
    `using ${normalizedSkill}`,
    `with ${normalizedSkill}`,
  ]
  return explicitMarkers.some((marker) => normalizedPrompt.includes(marker))
    || containsTokenishSkillName(normalizedPrompt, normalizedSkill)
}

function containsTokenishSkillName(prompt: string, skillName: string): boolean {
  let index = prompt.indexOf(skillName)
  while (index >= 0) {
    const before = index > 0 ? prompt.charCodeAt(index - 1) : null
    const afterIndex = index + skillName.length
    const after = afterIndex < prompt.length ? prompt.charCodeAt(afterIndex) : null
    if (isSkillBoundary(before) && isSkillBoundary(after)) return true
    index = prompt.indexOf(skillName, index + skillName.length)
  }
  return false
}

function isSkillBoundary(code: number | null): boolean {
  if (code === null) return true
  return !((code >= 48 && code <= 57) || (code >= 97 && code <= 122) || code === 45 || code === 95)
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`expected ${variant} response, received ${JSON.stringify(response)}`)
  }
  return response[variant] as T
}
