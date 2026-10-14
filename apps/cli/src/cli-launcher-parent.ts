import childProcess from "node:child_process"

type LauncherParentWatchOptions = {
  exit?: () => void
  inspect?: (pid: number) => boolean | null | Promise<boolean | null>
  platform?: NodeJS.Platform
}

let launcherParentExit: (message: string) => void = (message) => {
  process.stderr.write(message)
  process.exit(1)
}

// Registered synchronously during renderer composition. Before that, no
// terminal modes have been entered and the entry-point fallback can exit.
export function registerCliLauncherParentExit(exit: (message: string) => void): () => void {
  const previous = launcherParentExit
  launcherParentExit = exit
  return () => {
    if (launcherParentExit === exit) launcherParentExit = previous
  }
}

function exitLauncherChild(reason: string) {
  launcherParentExit(`chariox: ${reason}; closing TUI\n`)
}

// MP-08/MP-11: Linux uses the launcher's PR_SET_PDEATHSIG. macOS needs an
// asynchronous fallback, including while the renderer/runtime are loading.
export function watchCliLauncherParent(
  launcherPid: string | undefined,
  { exit, inspect = launcherExists, platform = process.platform }: LauncherParentWatchOptions = {},
): () => void {
  if (!launcherPid) return () => {}
  const expected = Number(launcherPid)
  if (!Number.isSafeInteger(expected) || expected <= 1) {
    throw new Error("invalid CLI launcher parent PID")
  }
  // A spawning BUN_BIN wrapper forks a grandchild without PR_SET_PDEATHSIG.
  // Only a direct Linux child can rely exclusively on the launcher's signal.
  if (platform === "linux" && process.ppid === expected) {
    // OpenTUI's SIGTERM handler destroys its renderer but leaves runtime timers
    // active. Finish exiting after restoration when PR_SET_PDEATHSIG fires.
    const terminate = exit ?? (() => exitLauncherChild("launcher termination received"))
    process.once("SIGTERM", terminate)
    return () => { process.off("SIGTERM", terminate) }
  }
  let stopped = false
  let pending = false
  const check = async () => {
    if (stopped || pending) return
    pending = true
    try {
      const alive = await inspect(expected)
      if (!stopped && alive === false) {
        stopped = true
        clearInterval(timer)
        if (exit) exit()
        else exitLauncherChild("launcher parent exited")
      }
    } catch {
      // A spawn failure or incomplete inspection is unknown, never parent death.
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

function launcherExists(pid: number): Promise<boolean | null> {
  // Bun caches process.ppid on macOS, and kill(pid, 0) accepts zombies and
  // recycled PIDs. Walk the live ancestry, allowing a spawning Bun wrapper.
  return new Promise((resolve) => {
    childProcess.execFile("/bin/ps", ["-axo", "pid=,ppid=,stat="], {
      encoding: "utf8",
      env: { PATH: "/usr/bin:/bin", LC_ALL: "C" },
      timeout: 1000,
    }, (error, stdout) => {
      // Spawn/timeout/signal errors are not a process-table observation. A
      // numeric nonzero ps exit may still carry a complete, useful snapshot.
      if (error && (error.signal || error.killed || typeof error.code !== "number")) {
        resolve(null)
        return
      }
      const rows = stdout.trim().split("\n").map((row) => row.trim().split(/\s+/))
      if (rows.some(([id, parent, state, extra]) => !/^\d+$/.test(id ?? "")
        || !/^\d+$/.test(parent ?? "") || !/^[A-Za-z]/.test(state ?? "") || extra)) {
        resolve(null)
        return
      }
      const processes = new Map(rows.map(([id, parent, state]) => [Number(id), { parent: Number(parent), state: state! }]))
      if (!processes.has(process.pid)) {
        resolve(null)
        return
      }
      const launcher = processes.get(pid)
      if (!launcher || launcher.state.startsWith("Z")) {
        resolve(false)
        return
      }
      const visited = new Set<number>()
      let ancestor = process.pid
      while (ancestor > 1 && !visited.has(ancestor)) {
        if (ancestor === pid) {
          resolve(true)
          return
        }
        visited.add(ancestor)
        const row = processes.get(ancestor)
        if (!row) {
          resolve(null)
          return
        }
        ancestor = row.parent
      }
      resolve(false)
    })
  })
}
