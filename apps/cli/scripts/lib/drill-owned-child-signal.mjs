// MP-08/MP-10/MP-11: signal only the ChildProcess returned by this drill's spawn.
export function signalOwnedDrillChild(child, signal) {
  return child.kill(signal)
}
