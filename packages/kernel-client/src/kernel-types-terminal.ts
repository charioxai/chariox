export type TerminalOperationContract = {
  id: string
  command?: string
  description: string
  search_aliases?: string[]
  intents?: string[]
  required_context?: string[]
  required_targets?: string[]
  input_schema: Record<string, unknown> | null
  result_kind: string
  mutation: boolean
  expected_projections?: string[]
  supported_surfaces?: string[]
  examples?: string[]
  parity_variants?: string[]
  presentation_only: boolean
}

export type TerminalOperationRegistry = {
  revision: string
  operations: TerminalOperationContract[]
}
