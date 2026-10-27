// MP-10: signal only a live direct child returned by this runner's spawn.
// Never accept process groups or a caller-supplied PID.
export function signalOwnedChild(child, signal = "SIGTERM") {
  if (!Number.isSafeInteger(child?.pid) || child.pid <= 1
    || child.exitCode !== null || child.signalCode !== null) return false
  return child.kill(signal)
}
