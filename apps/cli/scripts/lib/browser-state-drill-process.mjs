// The caller retains ChildProcess objects from its own spawn calls only.
export function signalBrowserStateChild(child, signal) {
  if (!Number.isSafeInteger(child?.pid) || child.pid <= 1
      || child.exitCode != null || child.signalCode != null) return false
  return child.kill(signal)
}
