export type CommandCenterItem = {
  id: string
  label: string
  description: string
  kind: "agent" | "command" | "group" | "provider" | "account" | "model" | "variant"
  value: string
  searchAliases?: string[] | undefined
}
