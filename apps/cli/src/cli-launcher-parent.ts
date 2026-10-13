import childProcess from "node:child_process"

// MP-08/MP-11: Linux uses the launcher's PR_SET_PDEATHSIG. macOS needs an
// asynchronous fallback, including while the renderer/runtime are loading.
export function watchCliLauncherParent(
  launcherPid: string | undefined,
  parentPid: () => number = () => process.ppid,
  exit: () => void = () => process.exit(1),
  parentExists: (pid: number) => boolean | Promise<boolean> = launcherExists,
  platform: NodeJS.Platform = process.platform,
): () => void {
  if (!launcherPid) return () => {}
  const expected = Number(launcherPid)
  if (!Number.isSafeInteger(expected) || expected <= 1) {
    throw new Error("invalid CLI launcher parent PID")
  }
  if (platform === "linux") return () => {}
  let stopped = false
  let pending = false
  const check = async () => {
    if (stopped || pending) return
    pending = true
    try {
      const alive = parentPid() === expected && await parentExists(expected)
      if (!stopped && !alive) {
        stopped = true
        clearInterval(timer)
        exit()
      }
    } finally {
      pending = false
    }
  }
  const timer = setInterval(() => { void check() }, 1000)
  timer.unref()
  void check()
  return () => {
    stopped = true
    clearInterval(timer)
  }
}

function launcherExists(pid: number): Promise<boolean> {
  // Bun caches process.ppid on macOS, and kill(pid, 0) accepts zombies and
  // recycled PIDs. Inspect current ancestry/state without blocking rendering.
  return new Promise((resolve) => {
    childProcess.execFile("/bin/ps", ["-o", "pid=,ppid=,stat=", "-p", `${process.pid},${pid}`], {
      encoding: "utf8",
      env: { PATH: "/usr/bin:/bin", LC_ALL: "C" },
      timeout: 1000,
    }, (error, stdout) => {
      if (error) {
        // A foreground-group signal can kill ps too. Retry on the next poll.
        resolve(Boolean(error.signal))
        return
      }
      const rows = stdout.trim().split("\n").map((row) => row.trim().split(/\s+/))
      resolve(rows.some(([id, parent]) => Number(id) === process.pid && Number(parent) === pid)
        && rows.some(([id, , state]) => Number(id) === pid && state && !state.startsWith("Z")))
    })
  })
}
