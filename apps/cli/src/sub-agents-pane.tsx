import type { BoxRenderable, ScrollBoxRenderable } from "@opentui/core"
import { createEffect, For, onCleanup } from "solid-js"
import type { SubAgentsController } from "@chariox/kernel-client/sub-agents-controller"
import { theme } from "./theme.js"

export type SubAgentsPaneProps = { controller: SubAgentsController; revision(): number; width: number; onFocus(agentId: string): void }

/** The Sub-agents view: one panel per sub-agent, status only. A held decision reads "waiting"; the prompt stays on the room agent. */
export function SubAgentsPane(props: SubAgentsPaneProps) {
  let list: ScrollBoxRenderable | undefined
  const boxes = new Map<string, BoxRenderable>()
  const controller = () => { props.revision(); return props.controller }
  createEffect(() => {
    const row = controller().focusedPanelId ? boxes.get(controller().focusedPanelId!) : undefined
    if (!row || !list || list.viewport.height <= 0) return
    const above = row.y - list.viewport.y
    const below = row.y + row.height - list.viewport.y - list.viewport.height
    if (above < 0) list.scrollBy(above)
    else if (below > 0) list.scrollBy(below)
  })
  return <box width={props.width} height="100%" minHeight={0} flexShrink={0} flexDirection="column" border={true} borderColor={theme.border} backgroundColor={theme.backgroundPanel}>
    <text fg={theme.text}>{`Sub-agents ${controller().panels().length}`}</text>
    <scrollbox ref={value => { list = value }} flexGrow={1} flexBasis={0} minHeight={0} stickyScroll={false}>
      <For each={controller().panels()}>{panel => {
        onCleanup(() => boxes.delete(panel.id))
        const active = () => controller().focusedPanelId === panel.id
        return <box ref={value => { boxes.set(panel.id, value) }} flexDirection="row" justifyContent="space-between" paddingLeft={panel.depth * 2} border={true}
          borderColor={active() ? theme.primary : theme.border} onMouseDown={() => props.onFocus(panel.id)}>
          <text fg={panel.dimmed ? theme.textMuted : theme.text} wrapMode="none">{panel.name}</text>
          <text fg={panel.dimmed ? theme.textMuted : theme.text} wrapMode="none">{panel.word}</text>
        </box>
      }}</For>
    </scrollbox>
  </box>
}
