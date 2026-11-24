// Local 1015: the pane feed (docs/PROTOCOL.md section 3.3.2.1). The kernel builds every word;
// a client or a provider terminal renders it and answers cards with RespondToInteraction.
export type GetPaneFeedRequest = { readonly session_id: string; readonly agent_id: string }

export type ReportPaneSettingsRequest = {
  readonly session_id: string
  readonly agent_id: string
  readonly model?: string
  readonly effort?: string
  readonly execution_mode?: "build" | "plan"
  readonly permission_level?: "required" | "yolo"
}

export type PaneFeed = {
  readonly title: string
  readonly identity: {
    readonly room_id: string
    readonly room_name: string
    readonly agent_id: string
    readonly agent_name: string
    readonly parents: readonly string[]
    readonly project_id: string
    readonly placement: string | null
  }
  readonly settings: {
    readonly provider: string
    readonly account: string | null
    readonly model: string | null
    readonly effort: string | null
    readonly execution_mode: "build" | "plan"
    readonly permission_level: "required" | "yolo"
  }
  readonly state: "idle" | "working" | "waiting_for_answer"
  readonly goal: string | null
  readonly cards: readonly PaneCard[]
  readonly stripes: readonly { readonly id: string; readonly kind: string; readonly text: string; readonly detail: string | null; readonly at_ms: number | null }[]
  readonly notices: readonly { readonly id: string; readonly title: string; readonly body: string | null; readonly at_ms: number }[]
  readonly link: string | null
}

export type PaneCard = {
  readonly id: string
  readonly asker_name: string | null
  readonly title: string | null
  readonly message: string
  readonly choices: readonly { readonly id: string; readonly label: string }[]
  readonly reply: { readonly id: string; readonly label: string; readonly placeholder: string | null; readonly secret: boolean } | null
  readonly text_terminal: boolean
  readonly requested_at_ms: number
  readonly open_elsewhere: string
}
