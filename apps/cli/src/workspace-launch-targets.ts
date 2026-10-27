import { execFile } from "node:child_process"
import { promisify } from "node:util"

const execFileAsync = promisify(execFile)

export async function inferWorkspaceTargetsFromLaunchDirectory(cwd: string): Promise<{ workspace: string; worktree: string }> {
  try {
    const [worktreeResult, commonDirResult] = await Promise.all([
      execFileAsync("git", ["rev-parse", "--show-toplevel"], { cwd }),
      execFileAsync("git", ["rev-parse", "--path-format=absolute", "--git-common-dir"], { cwd }),
    ])
    const worktree = worktreeResult.stdout.trim()
    const commonDir = commonDirResult.stdout.trim()
    if (!worktree) {
      return { workspace: cwd, worktree: cwd }
    }
    const workspace = commonDir.endsWith("/.git")
      ? commonDir.slice(0, -"/.git".length)
      : worktree
    return { workspace, worktree }
  } catch {
    return { workspace: cwd, worktree: cwd }
  }
}
