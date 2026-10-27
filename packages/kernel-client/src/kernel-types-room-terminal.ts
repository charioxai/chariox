// Wire shapes of Room terminals (kernel `room_terminal/types.rs`). Field names are the
// kernel's own snake_case: the shapes are pinned by the kernel's protocol shape tests.

export type TerminalActor =
  | { readonly kind: "user" }
  | { readonly kind: "agent"; readonly agent_id: string }

export type RoomTerminalDriver =
  | { readonly state: "free" }
  | { readonly state: "busy"; readonly driven_by: TerminalActor }

export type RoomTerminalOutcome =
  | { readonly reason: "program_exited"; readonly exit_code: number | null; readonly signal: string | null }
  | { readonly reason: "kernel_restarted" }

export type RoomTerminalStatus =
  | { readonly state: "running" }
  | { readonly state: "ended"; readonly outcome: RoomTerminalOutcome; readonly description: string }

export type RoomTerminalSummary = {
  readonly terminal_id: string
  readonly session_id: string
  readonly title: string
  readonly program: string
  readonly working_directory: string
  readonly cols: number
  readonly rows: number
  readonly status: RoomTerminalStatus
  readonly driver: RoomTerminalDriver
  readonly opened_by: TerminalActor
  readonly user_interacted: boolean
  readonly created_at_ms: number
  readonly output_offset: number
  readonly dropped_input_bytes?: number
  readonly host_agent?: string
}

export type RoomTerminalSnapshot = {
  readonly terminal: RoomTerminalSummary
  readonly cursor_row: number
  readonly cursor_col: number
  readonly screen_text: string
  readonly scrollback_text: string
  /** Base64 of the escape sequences that redraw the screen in a terminal emulator. */
  readonly screen_ansi_base64: string
  readonly output_offset: number
}

export type RoomTerminalOutput = {
  readonly terminal: RoomTerminalSummary
  readonly data_base64: string
  readonly next_offset: number
  /** The requested offset is no longer retained: read a snapshot to resync. */
  readonly gap: boolean
}

export type RoomTerminalInputResult = {
  readonly accepted: boolean
  readonly driver: RoomTerminalDriver
  readonly refusal?: string
}

export type RoomTerminalWatch =
  | { readonly kind: "finished" }
  | { readonly kind: "output_contains"; readonly text: string }
  | { readonly kind: "waiting_for_input" }

export type RoomTerminalEventKind =
  | { readonly kind: "finished"; readonly exit_code: number | null; readonly signal: string | null }
  | { readonly kind: "output_matched"; readonly text: string }
  | { readonly kind: "waiting_for_input" }

export type RoomTerminalWaitResult = {
  readonly fired: RoomTerminalEventKind | null
  readonly terminal: RoomTerminalSummary
  readonly screen_text: string
  readonly output_offset: number
}

export type ExecOutcome = { kind: "exited"; code: number } | { kind: "signaled"; signal: number } | { kind: "failed"; message: string }
export type ExecOutput = { chunks: readonly { cursor: number; stream: "stdout" | "stderr"; data_base64: string }[]; next_cursor: number; outcome: ExecOutcome | null }

export type RoomTerminalOperation =
  | { readonly operation: "exec_open"; readonly command: readonly string[] }
  | { readonly operation: "exec_write"; readonly exec_id: string; readonly offset: number; readonly data_base64: string }
  | { readonly operation: "exec_stdin_close"; readonly exec_id: string; readonly offset: number }
  | { readonly operation: "exec_read"; readonly exec_id: string; readonly after_cursor: number; readonly wait_ms?: number }
  | { readonly operation: "exec_cancel" | "exec_close"; readonly exec_id: string }
  | {
      readonly operation: "open"
      readonly cols: number
      readonly rows: number
      readonly title?: string
      readonly command?: readonly string[]
    }
  | { readonly operation: "list" }
  | { readonly operation: "snapshot"; readonly terminal_id: string }
  | { readonly operation: "input"; readonly terminal_id: string; readonly data_base64: string }
  | { readonly operation: "resize"; readonly terminal_id: string; readonly cols: number; readonly rows: number }
  | {
      readonly operation: "read_output"
      readonly terminal_id: string
      readonly after_offset: number
      readonly wait_ms?: number
    }
  | {
      readonly operation: "wait"
      readonly terminal_id: string
      readonly watches: readonly RoomTerminalWatch[]
      readonly after_offset?: number
      readonly timeout_ms: number
    }
  | { readonly operation: "close"; readonly terminal_id: string }

export type RoomTerminalRequest = {
  readonly session_id: string
  readonly acting_agent?: string
  readonly host_agent?: string
  readonly operation: RoomTerminalOperation
}

export type RoomTerminalResult =
  | { readonly result: "exec_opened"; readonly exec_id: string }
  | { readonly result: "exec_input"; readonly next_offset: number }
  | { readonly result: "exec_output"; readonly output: ExecOutput }
  | { readonly result: "exec_cancelled" | "exec_closed" }
  | { readonly result: "opened"; readonly terminal: RoomTerminalSummary }
  | { readonly result: "listed"; readonly terminals: readonly RoomTerminalSummary[] }
  | { readonly result: "snapshot"; readonly snapshot: RoomTerminalSnapshot }
  | { readonly result: "input"; readonly input: RoomTerminalInputResult }
  | { readonly result: "resized"; readonly terminal: RoomTerminalSummary }
  | { readonly result: "output"; readonly output: RoomTerminalOutput }
  | { readonly result: "waited"; readonly waited: RoomTerminalWaitResult }
  | { readonly result: "closed" }

export type RoomTerminalResponse = { readonly RoomTerminal: { readonly result: RoomTerminalResult } }
