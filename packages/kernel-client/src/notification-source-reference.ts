/** `/trigger notification <source> [prompt]`: which source the words name, and what is left as the prompt. */
import type { AgentTriggerNotificationSource, WorkflowNotificationSourceSummary } from "./kernel-types.js"
import { matchNotificationSource } from "./shell-workflow-notification-command.js"

export type NotificationSourceChoice = { source: AgentTriggerNotificationSource; prompt: string } | { error: string }

/**
 * A workflow source is named by its display name, `kernel/display name` or id, and the name may contain
 * spaces, so the longest run of leading words that names exactly one source wins. An App event is
 * `app:<installation>/<event>`.
 */
export function chooseNotificationSource(words: string[], sources: WorkflowNotificationSourceSummary[]): NotificationSourceChoice {
  const first = words[0]
  if (!first) return { error: "name a source: /workflow trigger notification list shows them" }
  if (first.startsWith("app:")) {
    const [installation, ...event] = first.slice("app:".length).split("/")
    if (!installation || event.length === 0 || !event.join("/")) return { error: "an App event is app:<installation>/<event>" }
    return { source: { app: { installation_id: installation, event_name: event.join("/") } }, prompt: words.slice(1).join(" ") }
  }
  for (let count = words.length; count >= 1; count--) {
    const matches = matchNotificationSource(sources, words.slice(0, count).join(" "))
    if (matches.length > 1) return { error: `"${words.slice(0, count).join(" ")}" names more than one source; use kernel/name or the source id` }
    const source = matches[0]
    if (!source) continue
    if (!source.available) return { error: `${source.display_name} is not available: its kernel is offline` }
    return { source: { workflow: { source_id: source.source_id } }, prompt: words.slice(count).join(" ") }
  }
  return { error: `no such source "${words.join(" ")}"; /workflow trigger notification list shows them` }
}
