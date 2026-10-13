// MP-08/MP-11: macOS has no PR_SET_PDEATHSIG. Watch the exact launcher parent
// from the entry point, including while the renderer/runtime are still loading.
export function watchCliLauncherParent(
  launcherPid: string | undefined,
  parentPid: () => number = () => process.ppid,
  exit: () => void = () => process.exit(1),
  parentExists: (pid: number) => boolean = launcherExists,
): () => void {
  if (!launcherPid) return () => {}
  const expected = Number(launcherPid)
  if (!Number.isSafeInteger(expected) || expected <= 1) {
    throw new Error("invalid CLI launcher parent PID")
  }
  const check = () => {
    if (parentPid() !== expected || !parentExists(expected)) exit()
  }
  check()
  const timer = setInterval(check, 100)
  timer.unref()
  return () => clearInterval(timer)
}

function launcherExists(pid: number): boolean {
  // Bun caches process.ppid on macOS. Query only the validated launcher PID;
  // signal 0 checks existence without delivering a signal to that process.
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    return (error as NodeJS.ErrnoException).code !== "ESRCH"
  }
}
