// MP-08/MP-11: macOS has no PR_SET_PDEATHSIG. Watch the exact launcher parent
// from the entry point, including while the renderer/runtime are still loading.
export function watchCliLauncherParent(
  launcherPid: string | undefined,
  parentPid: () => number = () => process.ppid,
  exit: () => void = () => process.exit(1),
): () => void {
  if (!launcherPid) return () => {}
  const expected = Number(launcherPid)
  if (!Number.isSafeInteger(expected) || expected <= 1) {
    throw new Error("invalid CLI launcher parent PID")
  }
  const check = () => {
    if (parentPid() !== expected) exit()
  }
  check()
  const timer = setInterval(check, 100)
  timer.unref()
  return () => clearInterval(timer)
}
