export type CommandCenterItem = {
  id: string
  label: string
  description: string
  skillSelection?: import("@chariox/kernel-client/kernel-types-skills-v2").SkillSelection
  kind: "skill" | "skill-guidance" | "agent" | "command" | "group" | "provider" | "account" | "model" | "variant"
  value: string
  tone?: "warning" | "danger" | undefined
  searchAliases?: string[] | undefined
}
