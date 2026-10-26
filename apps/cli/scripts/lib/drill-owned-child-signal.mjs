// MP-08/MP-10/MP-11: signal only the ChildProcess returned by this drill's spawn.
export function signalOwnedDrillChild(child, signal) {
  if (!Number.isSafeInteger(child?.pid) || child.pid <= 1) {
    throw new Error('MP-08/MP-10/MP-11 unsafe cleanup PID')
  }
  if (child.exitCode !== null || child.signalCode !== null) return false
  return child.kill(signal)
}
