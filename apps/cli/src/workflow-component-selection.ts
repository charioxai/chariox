export type WorkflowComponentSelection =
  | { kind: "workflow" }
  | { kind: "node"; id: string }
  | { kind: "edge"; id: string }
  | { kind: "endpoint"; id: string }
