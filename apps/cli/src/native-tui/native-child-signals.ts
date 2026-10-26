// MP-08/MP-10/MP-11: signal only a live child created by the native launcher.
import type { ChildProcess } from "node:child_process"
export function signalNativeChild(child: Pick<ChildProcess, "pid" | "exitCode" | "signalCode" | "kill">,
  signal: NodeJS.Signals): boolean {
  if (typeof child.pid !== "number" || !Number.isSafeInteger(child.pid) || child.pid <= 1
    || child.exitCode != null || child.signalCode != null) return false
  return child.kill(signal)
}
