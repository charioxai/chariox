import { runAgentSettingsCommand } from "./agent-settings-command.js"
import { runAgentDeadlineCommand } from "./agent-deadline-command.js"
import { runAccessCommand } from "./access-command.js"
import { runInteractionCommand } from "./interaction-command.js"
import { runKernelsCommand } from "./kernels-command.js"
import { runSecretCommand } from "./secret-command.js"

/**
 * The verbs an outside agent calls: they print JSON over the kernel's unix socket and never
 * draw the TUI, so they run before the rendering stack loads (its module graph was most of
 * their start-up time).
 */
export const AGENT_VERB_NAMES = ["access", "kernels", "request", "watch", "files", "events", "guide", "terminal", "exec", "secret", "interaction", "agent"]

export async function runAgentVerb(argv: readonly string[]): Promise<boolean> {
  return (await runAgentSettingsCommand(argv)) || (await runAgentDeadlineCommand([...argv])) || (await runAccessCommand([...argv])) || (await runSecretCommand([...argv])) || (await runInteractionCommand([...argv])) || (await runKernelsCommand([...argv]))
}
