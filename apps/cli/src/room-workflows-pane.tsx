import type { TextareaRenderable, ScrollBoxRenderable, BoxRenderable } from "@opentui/core"
import { createEffect, For, Show, onCleanup } from "solid-js"
import type { RoomWorkflowsPaneController } from "./room-workflows-pane-controller.js"
import { roomWorkflowRunTargets } from "@chariox/kernel-client/room-workflows"
import { PROMPT_KEYBINDINGS } from "./cli-runtime-tuning.js"
import { theme } from "./theme.js"

export type RoomWorkflowsPaneProps = { controller: RoomWorkflowsPaneController; revision(): number; width: number; onAgents(): void; onFocus(): void }
export function RoomWorkflowsPane(props: RoomWorkflowsPaneProps) {
 let input: TextareaRenderable | undefined
 let list: ScrollBoxRenderable | undefined
 const rowBoxes = new Map<string, BoxRenderable>()
 const controller = () => { props.revision(); return props.controller }
 const record = () => controller().record
 createEffect(() => {
   const draft = record()?.draft ?? ""
   const selected = record()?.selectedKey
   const row = selected ? rowBoxes.get(selected) : undefined
   if (row && list && list.viewport.height > 0) {
     const above = row.y - list.viewport.y
     const below = row.y + row.height - list.viewport.y - list.viewport.height
     if (above < 0) list.scrollBy(above)
     else if (below > 0) list.scrollBy(below)
   }
   if (input && input.plainText !== draft) input.setText(draft)
   if (controller().focused && controller().visible && input && !input.focused) input.focus()
 })
 return <box width={props.width} height="100%" minHeight={0} flexShrink={0} flexDirection="column" border={true} borderColor={theme.border} backgroundColor={theme.backgroundPanel} onMouseDown={props.onFocus}>
   <box flexDirection="row" flexShrink={0}>
     <text fg={theme.text}>Workflows </text>
     <text fg={theme.textMuted} onMouseDown={() => controller().manage()}>[Manage] </text>
     <text fg={theme.textMuted} onMouseDown={props.onAgents}>[Agents] </text>
     <text fg={theme.textMuted} onMouseDown={() => { controller().close(); props.onAgents() }}>[×]</text>
   </box>
   <Show when={!record()?.fresh}><text fg={theme.textMuted}>Offline · stale</text></Show>
   <scrollbox ref={value => { list = value }} flexGrow={1} flexBasis={0} minHeight={0} stickyScroll={false}>
     <Show when={!controller().rows.length}><text fg={theme.textMuted}>No entry points yet. Add one in Manage.</text></Show>
     <For each={controller().rows}>{(row, index) => { onCleanup(() => rowBoxes.delete(row.key)); return <box ref={value => { rowBoxes.set(row.key, value) }} flexDirection="column" paddingBottom={1} onMouseDown={() => { controller().select(index()); props.onFocus() }}>
       <text fg={row.key === record()?.selectedKey ? theme.text : theme.textMuted}>{`${row.key === record()?.selectedKey ? "›" : " "} ${row.workflow.label} / ${row.endpoint.label}`}</text>
       <text fg={theme.textMuted}>{`${row.workflow.state} · ${row.workflow.running_count} running · ${row.workflow.paused_count} paused · ${row.workflow.queued_count} queued`}</text>

         <For each={["pause", "resume", "stop"] as const}>{action => <Show when={roomWorkflowRunTargets(row.workflow, action).length}>
           <text fg={theme.textMuted} onMouseDown={() => { controller().select(index()); void controller().control(action) }}>{`${action[0]!.toUpperCase()}${action.slice(1)} current runs (${roomWorkflowRunTargets(row.workflow, action).length})`}</text>
         </Show>}</For>
     </box> }}</For>
   </scrollbox>
   <box flexShrink={0} flexDirection="column">
     <text fg={theme.textMuted}>{controller().selected ? `Start ${controller().selected!.workflow.label} / ${controller().selected!.endpoint.label}` : "Select an entry point"}</text>
     <Show when={controller().selected && !controller().selected!.endpoint.can_start}><text fg={theme.textMuted}>{controller().selected!.endpoint.start_disabled_reason ?? "Only the endpoint owner can start"}</text></Show>
     <textarea ref={value => { input = value; value.setText(record()?.draft ?? "") }} minHeight={2} maxHeight={5} textColor={theme.text} focusedTextColor={theme.text} keyBindings={PROMPT_KEYBINDINGS}
       onContentChange={() => { if (input) controller().draft(input.plainText) }} onSubmit={() => { void controller().start() }} />
     <text fg={theme.textMuted} onMouseDown={() => { void controller().start() }}>{record()?.busy ? "Submitting…" : "[Start · Enter]"}</text>
     <Show when={record()?.message}><text fg={theme.textMuted}>{record()?.message}</text></Show>
     <text fg={theme.textMuted}>Ctrl+↑/↓ select · ^P pause · ^R resume</text>
     <text fg={theme.textMuted}>^S stop · ^G manage · Tab agents</text>
   </box>
 </box>
}
