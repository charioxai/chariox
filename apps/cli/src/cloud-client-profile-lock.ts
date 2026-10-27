import { spawn } from "node:child_process"
import { constants } from "node:fs"
import { open } from "node:fs/promises"

/** An OS lock is released on process death. It avoids stale-file races that
 * could rotate one profile twice and trigger refresh-family reuse detection. */
export async function withCloudClientProfileLock<T>(filePath: string, operation: () => Promise<T>): Promise<T> {
  const handle = await open(filePath, constants.O_RDWR | constants.O_CREAT | constants.O_NOFOLLOW, 0o600)
  const stat = await handle.stat()
  if (!stat.isFile() || stat.nlink !== 1 || (stat.mode & 0o777) !== 0o600 || (process.getuid && stat.uid !== process.getuid())) {
    await handle.close()
    throw new Error("unsafe Cloud profile lock")
  }
  const hold = "printf locked; cat >/dev/null"
  const child = process.platform === "darwin"
    ? spawn("/usr/bin/lockf", ["-k", "-t", "30", filePath, "/bin/sh", "-c", hold], { stdio: ["pipe", "pipe", "ignore"] })
    : spawn("/bin/sh", ["-c", `flock -x -w 30 3 || exit; ${hold}`], { stdio: ["pipe", "pipe", "ignore", handle.fd] })
  child.stdin!.on("error", () => {})
  const exited = new Promise<void>(resolve => child.once("close", () => resolve()))
  try {
    await new Promise<void>((resolve, reject) => {
      child.once("error", () => reject(new Error("Cloud profile OS lock is unavailable")))
      child.once("exit", () => reject(new Error("Cloud profile rotation lock was not acquired")))
      child.stdout!.once("data", () => resolve())
    })
    return await operation()
  } finally {
    child.stdin!.end()
    await exited
    await handle.close()
  }
}
