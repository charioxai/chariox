// MP-08 / MP-10 / MP-11: no client-owned resolver, bodies or capability grants.
import type { SkillsV2Request, SkillsV2Response, SkillCatalogMutation } from "./kernel-types-skills-v2.js"
export { SKILLS_MIN_LOCAL_PROTOCOL, SKILLS_MIN_PEER_PROTOCOL } from "./kernel-types-skills-v2.js"

export function skillsV2Request(request: SkillsV2Request): { SkillsV2: SkillsV2Request } {
  return { SkillsV2: request }
}
export function skillCatalogRequest(projectId: string | null = null, agentId: string | null = null) {
  return skillsV2Request({ action: "catalog", project_id: projectId, agent_id: agentId })
}
export function mutateSkillCatalogRequest(mutation: SkillCatalogMutation) {
  return skillsV2Request({ action: "mutate", mutation })
}
export function previewSkillCatalogRequest(mutation: SkillCatalogMutation) {
  return skillsV2Request({ action: "preview", mutation })
}
export function skillsV2Response(response: { SkillsV2: { result: SkillsV2Response } }): SkillsV2Response {
  return response.SkillsV2.result
}
