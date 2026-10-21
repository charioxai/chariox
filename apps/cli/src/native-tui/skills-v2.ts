// MP-08 / MP-10 / MP-11: native clients project home state, never read skill bodies.
import type { LocalIpcClient } from "../ipc.js"
import { getSessionStateRequest } from "../ipc-requests.js"
import { skillCatalogRequest } from "@chariox/kernel-client/ipc-skills-v2-requests"
import type { SkillSelection, SkillsV2Response } from "@chariox/kernel-client/kernel-types-skills-v2"

type Catalog = Extract<SkillsV2Response, { kind: "catalog" }>
export async function nativeSkillCatalog(client: LocalIpcClient, sessionId: string, agentId: string): Promise<Catalog> {
  const state = await client.send<{ SessionState: { session: { project_id: string | null } } }>(getSessionStateRequest(sessionId))
  if (!state.SessionState) throw new Error("kernel did not return native skill session context")
  const response = await client.send<{ SkillsV2: { result: SkillsV2Response } }>(
    skillCatalogRequest(state.SessionState.session.project_id, agentId))
  const result = response.SkillsV2?.result
  if (result?.kind !== "catalog") throw new Error("kernel did not return the Skills v2 catalog; upgrade or refresh Skills")
  return result
}

// `offered` holds names the user picked from the upstream native completion.
// Plain `$name` stays under Chariox precedence; an offered native choice remains
// independent of an Off Chariox namesake.
export function codexSkillSelections(names: string[], catalog: Catalog, offered: ReadonlySet<string> = new Set()): SkillSelection[] {
  return names.map(name => {
    const row = catalog.skills.find(row => row.skill.name.toLowerCase() === name.toLowerCase())
    const native = nativeEntries(catalog, name)
    if (row?.effective_state === "off" && !(offered.has(name) && native.length === 1)) {
      throw new Error(`skill $${name} is Off; enable it in Skills before selecting it`)
    }
    if (row && row.effective_state !== "off") {
      return { source: "chariox", skill_id: row.skill.id, name: row.skill.name, catalog_revision: catalog.revision }
    }
    if (native.length !== 1) throw new Error(`native skill $${name} is unavailable or ambiguous; refresh Skills and select again`)
    return native[0]!.selection
  })
}

const nativeEntries = (catalog: Catalog, name: string) => catalog.native.filter(entry => entry.exposure === "confirmed"
  && entry.selection.source === "native" && entry.selection.provider === "codex" && entry.selection.name === name)

// Only native completion receives the projection. Kernel inventory always sees
// the upstream harness result, before any Chariox rows. Synthetic paths are UI
// handles and must never enter provider execution or filesystem operations.
export function projectCodexSkillList(result: Record<string, unknown>, catalog: Catalog): Record<string, unknown> {
  if (!Array.isArray(result.data)) throw new Error("Codex skills/list response has no cwd inventory")
  return { ...result, data: result.data.map((item: Record<string, unknown>) => {
    const skills = Array.isArray(item.skills) ? [...item.skills] : []
    for (const row of catalog.skills) {
      if (row.effective_state === "off" || row.delivery.kind === "provided_by") continue
      const name = row.skill.name
      const index = skills.findIndex(skill => typeof skill.name === "string" && skill.name.toLowerCase() === name.toLowerCase())
      const projected = {
        name, description: row.skill.description,
        shortDescription: row.skill.short_description,
        path: `chariox-skill://${encodeURIComponent(row.skill.id)}/SKILL.md`,
        scope: "user", enabled: true,
        interface: { displayName: name, shortDescription: row.skill.short_description ?? row.skill.description },
      }
      if (index >= 0) skills[index] = projected
      else skills.push(projected)
    }
    return { ...item, skills }
  }) }
}
