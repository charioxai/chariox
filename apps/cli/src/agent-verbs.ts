import { runAccessCommand } from "./access-command.js"
import { runKernelsCommand } from "./kernels-command.js"

/**
 * The verbs an outside agent calls: they print JSON over the kernel's unix socket and never
 * draw the TUI, so they run before the rendering stack loads (its module graph was most of
 * their start-up time).
 */
export const AGENT_VERB_NAMES = ["access", "kernels", "request", "watch", "files", "events", "guide", "terminal", "exec"]

export async function runAgentVerb(argv: readonly string[]): Promise<boolean> {
  return (await runAccessCommand([...argv])) || (await runKernelsCommand([...argv]))
}
