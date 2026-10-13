import { execFileSync } from "node:child_process"

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
  // Bun caches process.ppid on macOS, and kill(pid, 0) accepts zombies and
  // recycled PIDs. Read current ancestry and state in one process-table query.
  try {
    const rows = execFileSync("/bin/ps", ["-o", "pid=,ppid=,stat=", "-p", `${process.pid},${pid}`], {
      encoding: "utf8",
      timeout: 1000,
      stdio: ["ignore", "pipe", "ignore"],
    }).trim().split("\n").map((row) => row.trim().split(/\s+/))
    return rows.some(([id, parent]) => Number(id) === process.pid && Number(parent) === pid)
      && rows.some(([id, , state]) => Number(id) === pid && state && !state.startsWith("Z"))
  } catch {
    return false
  }
}
