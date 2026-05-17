export type CommandCenterItem = {
  id: string
  label: string
  description: string
  kind: "command" | "group" | "provider" | "model" | "variant"
  value: string
  searchAliases?: string[] | undefined
}
