import { execFile } from "node:child_process"
import { stat } from "node:fs/promises"
import { basename, dirname, resolve as resolvePath } from "node:path"
import { promisify } from "node:util"

const execFileAsync = promisify(execFile)

export type LocalGitWorktreeOptions = {
  baseDirectory: string
  targetDirectory?: string | undefined
  branch?: string | undefined
  fromRef?: string | undefined
}

export type PlacementOptions = {
  positional: string[]
  directory?: string | undefined
  gitWorktree?: string | undefined
  branch?: string | undefined
  fromRef?: string | undefined
  kernelRef?: string | undefined
  sliceRef?: string | undefined
  sliceDisplayMode?: "headless" | "headed" | undefined
}

export type ShellPlacementDeps = {
  prepareLocalGitWorktree?: ((options: LocalGitWorktreeOptions) => Promise<string>) | undefined
  resolveExistingDirectory?: ((directory: string, baseDirectory: string, label: string) => Promise<string>) | undefined
}

export function parsePlacementOptions(args: string[], allowMachine: boolean): { options: PlacementOptions; error?: string } {
  const options: PlacementOptions = { positional: [] }
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    const next = args[index + 1]
    if ((arg === "--dir" || arg === "--directory") && next) {
      options.directory = next
      index += 1
    } else if (arg === "--worktree" && next) {
      options.gitWorktree = next
      index += 1
    } else if (arg === "--branch" && next) {
      options.branch = next
      index += 1
    } else if (arg === "--from" && next) {
      options.fromRef = next
      index += 1
    } else if (arg === "--kernel" && next && allowMachine) {
      options.kernelRef = next
      index += 1
    } else if (arg === "--slice" && next && allowMachine) {
      options.sliceRef = next
      index += 1
    } else if (arg === "--slice-display" && next && allowMachine) {
      if (next !== "headless" && next !== "headed") {
        return { options, error: "--slice-display must be headless or headed" }
      }
      options.sliceDisplayMode = next
      index += 1
    } else if (arg?.startsWith("--")) {
      return { options, error: `unknown or incomplete option: ${arg}` }
    } else if (arg) {
      options.positional.push(arg)
    }
  }
  if (options.directory && options.gitWorktree) {
    return { options, error: "use either --dir or --worktree, not both" }
  }
  if ((options.branch || options.fromRef) && !options.gitWorktree) {
    return { options, error: "--branch/--from require --worktree" }
  }
  if (options.kernelRef && options.sliceRef) {
    return { options, error: "use either --kernel or --slice, not both" }
  }
  if (options.sliceDisplayMode && options.sliceRef !== "new") {
    return { options, error: "--slice-display requires --slice new" }
  }
  return { options }
}

export async function resolveShellPlacement(
  options: PlacementOptions,
  baseDirectory: string,
  label: string,
  deps: ShellPlacementDeps,
): Promise<string | undefined> {
  if (options.kernelRef) {
    return undefined
  }
  const positionalDirectory = options.positional.length === 1 && !options.directory && !options.gitWorktree
    ? options.positional[0]
    : undefined
  if (positionalDirectory || options.directory) {
    const directory = positionalDirectory ?? options.directory!
    const resolver = deps.resolveExistingDirectory ?? defaultResolveExistingDirectory
    return resolver(directory, baseDirectory, label)
  }
  if (options.gitWorktree) {
    const prepare = deps.prepareLocalGitWorktree ?? defaultPrepareLocalGitWorktree
    return prepare({
      baseDirectory,
      targetDirectory: options.gitWorktree,
      branch: options.branch,
      fromRef: options.fromRef,
    })
  }
  return undefined
}

async function defaultResolveExistingDirectory(directory: string, baseDirectory: string, label: string): Promise<string> {
  const resolved = resolvePath(baseDirectory, directory)
  const details = await stat(resolved)
  if (!details.isDirectory()) {
    throw new Error(`${label} is not a directory: ${resolved}`)
  }
  return resolved
}

async function defaultPrepareLocalGitWorktree(options: LocalGitWorktreeOptions): Promise<string> {
  const baseDirectory = resolvePath(options.baseDirectory)
  const repoRoot = (await runGit(baseDirectory, ["rev-parse", "--show-toplevel"])).trim()
  if (!repoRoot) {
    throw new Error(`git did not report a repository root for ${baseDirectory}`)
  }
  const fromRef = options.fromRef ?? "HEAD"
  const targetDirectory = options.targetDirectory
    ? resolvePath(baseDirectory, options.targetDirectory)
    : resolvePath(dirname(repoRoot), `${basename(repoRoot)}-${slugifyGitBranch(options.branch ?? fromRef)}`)
  const args = options.branch
    ? ["worktree", "add", "-b", options.branch, targetDirectory, fromRef]
    : ["worktree", "add", targetDirectory, fromRef]
  await runGit(repoRoot, args)
  const details = await stat(targetDirectory)
  if (!details.isDirectory()) {
    throw new Error(`created git worktree is not a directory: ${targetDirectory}`)
  }
  return targetDirectory
}

async function runGit(cwd: string, args: string[]): Promise<string> {
  try {
    const { stdout } = await execFileAsync("git", args, { cwd })
    return stdout
  } catch (error) {
    const detail = error && typeof error === "object" && "stderr" in error
      ? String((error as { stderr?: unknown }).stderr ?? "").trim()
      : ""
    const message = detail || (error instanceof Error ? error.message : String(error))
    throw new Error(`git ${args.join(" ")} failed in ${cwd}: ${message}`)
  }
}

function slugifyGitBranch(value: string): string {
  return value
    .trim()
    .replace(/[^A-Za-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    || "worktree"
}
