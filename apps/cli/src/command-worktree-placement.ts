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

export type RemoteGitWorktreePlacement = {
  target_directory?: string | null
  branch?: string | null
  from_ref?: string | null
}

export type PlacementParseResult = {
  positional: string[]
  directory?: string | undefined
  machineRef?: string | undefined
  kernelRef?: string | undefined
  sliceRef?: string | undefined
  sliceDisplayMode?: "headless" | "headed" | undefined
  externalSessionId?: string | undefined
  metaagent?: boolean | undefined
  gitWorktree?: string | undefined
  branch?: string | undefined
  fromRef?: string | undefined
  error?: string | undefined
}

export type LocalPlacementOptions = {
  directory?: string | undefined
  gitWorktree?: string | undefined
  branch?: string | undefined
  fromRef?: string | undefined
  machineRef?: string | undefined
  kernelRef?: string | undefined
  label: string
}

export type LocalPlacementContext = {
  baseDirectory: string
}

export function parsePlacementOptions(
  args: string[],
  commandName: string,
  allowMachine: boolean,
): PlacementParseResult {
  const positional: string[] = []
  let directory: string | undefined
  let machineRef: string | undefined
  let kernelRef: string | undefined
  let sliceRef: string | undefined
  let sliceDisplayMode: "headless" | "headed" | undefined
  let externalSessionId: string | undefined
  let gitWorktree: string | undefined
  let branch: string | undefined
  let fromRef: string | undefined
  let error: string | undefined

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (!arg) {
      continue
    }
    if (arg === "--dir" || arg === "--directory") {
      const value = args[index + 1]
      if (!value || value.startsWith("--")) {
        error = `usage: ${commandName} --dir <directory>`
        break
      }
      directory = value
      index += 1
      continue
    }
    if (arg === "--worktree") {
      const value = args[index + 1]
      if (!value || value.startsWith("--")) {
        error = `usage: ${commandName} --worktree <directory> [--branch <branch>] [--from <ref>]`
        break
      }
      gitWorktree = value
      index += 1
      continue
    }
    if (arg === "--branch") {
      const value = args[index + 1]
      if (!value || value.startsWith("--")) {
        error = `usage: ${commandName} [--worktree <directory>] --branch <branch> [--from <ref>]`
        break
      }
      branch = value
      index += 1
      continue
    }
    if (arg === "--from") {
      const value = args[index + 1]
      if (!value || value.startsWith("--")) {
        error = `usage: ${commandName} [--worktree <directory>] [--branch <branch>] --from <ref>`
        break
      }
      fromRef = value
      index += 1
      continue
    }
    if ((arg === "--machine" || arg === "--kernel") && allowMachine) {
      const value = args[index + 1]
      if (!value || value.startsWith("--")) {
        error = `usage: /agent spawn [alias] [model] ${arg} <ref> --dir <remote-directory>`
        break
      }
      if (arg === "--machine") {
        machineRef = value
      } else {
        kernelRef = value
      }
      index += 1
      continue
    }
    if (arg === "--slice" && allowMachine) {
      const value = args[index + 1]
      if (!value || value.startsWith("--")) {
        error = "usage: /agent spawn [alias] [model] --slice off|new|new:headless|new:headed|<slice-ref>"
        break
      }
      sliceRef = value === "off" ? undefined : value
      index += 1
      continue
    }
    if (arg === "--slice-display" && allowMachine) {
      const value = args[index + 1]
      if (value !== "headless" && value !== "headed") {
        error = "usage: /agent spawn [alias] [model] --slice-display headless|headed"
        break
      }
      sliceDisplayMode = value
      index += 1
      continue
    }
    if (arg === "--unattached-agent" && allowMachine) {
      const value = args[index + 1]
      if (!value || value.startsWith("--")) {
        error = "usage: /agent spawn --unattached-agent <external-session-id>"
        break
      }
      externalSessionId = value
      index += 1
      continue
    }
    if (arg.startsWith("--")) {
      error = `unknown ${commandName} option ${arg}`
      break
    }
    positional.push(arg)
  }

  const gitRequested = Boolean(gitWorktree || branch || fromRef)
  if (!error && directory && gitRequested) {
    error = `usage: ${commandName} uses either --dir or --worktree/--branch, not both`
  }
  if (!error && machineRef && kernelRef) {
    error = "usage: /agent spawn uses either --machine or --kernel, not both"
  }
  const remoteRef = machineRef ?? kernelRef
  const sliceCreatesPlacement = sliceRef === "new" || sliceRef === "new:headless" || sliceRef === "new:headed"
  if (!error && remoteRef && sliceRef && !sliceCreatesPlacement) {
    error = "usage: /agent spawn uses either --machine/--kernel or a reusable --slice, not both"
  }
  if (!error && sliceDisplayMode && !sliceCreatesPlacement) {
    error = "usage: /agent spawn --slice-display requires --slice new"
  }
  if (!error && sliceRef && !sliceCreatesPlacement && (directory || gitRequested)) {
    error = "usage: /agent spawn --slice <slice-ref> does not accept --dir or --worktree"
  }
  if (!error && externalSessionId && (directory || gitRequested || machineRef || kernelRef || sliceRef)) {
    error = "usage: /agent spawn --unattached-agent <external-session-id> does not accept placement options"
  }
  if (!error && remoteRef && gitRequested && !gitWorktree) {
    error = "usage: /agent spawn [alias] [model] --machine/--kernel <ref> --worktree <remote-directory> --branch <branch>"
  }
  if (!error && remoteRef && gitRequested && !branch) {
    error = "usage: /agent spawn [alias] [model] --machine/--kernel <ref> --worktree <remote-directory> --branch <branch>"
  }

  return {
    positional,
    directory,
    machineRef,
    kernelRef,
    sliceRef,
    sliceDisplayMode,
    externalSessionId,
    gitWorktree,
    branch,
    fromRef,
    error,
  }
}

export function parseAgentSpawnOptions(args: string[]): PlacementParseResult {
  const metaagent = args.includes("--meta") || args.includes("--metaagent")
  const spawnArgs = args.filter((arg) => arg !== "--meta" && arg !== "--metaagent")
  const parsed = parsePlacementOptions(spawnArgs, "/agent spawn", true)
  let error = parsed.error
  if (!error && parsed.positional.length > 2) {
    error = "usage: /agent spawn [alias] [model] [--dir <directory>] [--worktree <directory> --branch <branch>] [--machine <machine-ref>|--kernel <kernel-ref>] [--slice off|new|new:headless|new:headed|<slice-ref>] [--unattached-agent <external-session-id>]"
  }
  if (!error && metaagent) {
    error = "creating separate metaagents is deprecated; send /meta <task> to a regular agent to enter meta mode"
  }
  return {
    ...parsed,
    metaagent: false,
    error,
  }
}

export async function resolveExistingLocalDirectory(
  directory: string,
  baseDirectory: string,
  label: string,
): Promise<string> {
  const resolved = resolvePath(baseDirectory, directory)
  const details = await stat(resolved)
  if (!details.isDirectory()) {
    throw new Error(`${label} is not a directory: ${resolved}`)
  }
  return resolved
}

export async function prepareLocalGitWorktree(
  options: LocalGitWorktreeOptions,
  override?: (options: LocalGitWorktreeOptions) => Promise<string>,
): Promise<string> {
  if (override) {
    return override(options)
  }
  return defaultPrepareLocalGitWorktree(options)
}

export async function resolveLocalPlacement(
  options: LocalPlacementOptions,
  context: LocalPlacementContext,
): Promise<string | undefined> {
  if (options.directory) {
    if (options.machineRef || options.kernelRef) {
      return options.directory
    }
    return resolveExistingLocalDirectory(options.directory, context.baseDirectory, options.label)
  }
  return undefined
}

export function gitWorktreePlacementFromParse(
  parsed: Pick<PlacementParseResult, "gitWorktree" | "branch" | "fromRef">,
): RemoteGitWorktreePlacement | undefined {
  if (!parsed.gitWorktree && !parsed.branch && !parsed.fromRef) {
    return undefined
  }
  return {
    target_directory: parsed.gitWorktree ?? null,
    branch: parsed.branch ?? null,
    from_ref: parsed.fromRef ?? null,
  }
}

export function worktreeAliasConfigPath(worktreePath: string): string {
  const encoded = Buffer.from(worktreePath).toString("base64url")
  return `ui.worktree_aliases.${encoded}`
}

export function suggestNamedWorktreePath(baseDirectory: string, branch: string, explicitPath?: string): string {
  if (explicitPath?.trim()) {
    return resolvePath(baseDirectory, explicitPath)
  }
  return resolvePath(dirname(baseDirectory), defaultWorktreeDirectoryBase(basename(baseDirectory), branch))
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
    : resolvePath(dirname(repoRoot), defaultWorktreeDirectoryBase(basename(repoRoot), options.branch ?? fromRef))

  let args: string[]
  if (options.branch) {
    const branchExists = await gitBranchExists(repoRoot, options.branch)
    args = branchExists
      ? ["worktree", "add", targetDirectory, options.branch]
      : ["worktree", "add", "-b", options.branch, targetDirectory, fromRef]
  } else {
    args = ["worktree", "add", targetDirectory, fromRef]
  }

  await runGit(repoRoot, args)
  const details = await stat(targetDirectory)
  if (!details.isDirectory()) {
    throw new Error(`created git worktree is not a directory: ${targetDirectory}`)
  }
  return targetDirectory
}

async function gitBranchExists(repoRoot: string, branch: string): Promise<boolean> {
  try {
    await runGit(repoRoot, ["rev-parse", "--verify", "--quiet", `refs/heads/${branch}`])
    return true
  } catch {
    return false
  }
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

export function defaultWorktreeDirectoryBase(repoName: string, branchOrRef: string): string {
  const repoSlug = slugifyGitBranch(repoName)
  const branchLeaf = branchOrRef.split("/").filter(Boolean).at(-1) ?? branchOrRef
  const branchSlug = slugifyGitBranch(branchLeaf)
  const branchSlugLower = branchSlug.toLowerCase()
  const repoSlugLower = repoSlug.toLowerCase()
  if (branchSlugLower === repoSlugLower || branchSlugLower.startsWith(`${repoSlugLower}-`)) {
    return branchSlug
  }
  return `${repoSlug}-${branchSlug}`
}

function slugifyGitBranch(value: string): string {
  return value
    .trim()
    .replace(/[^A-Za-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    || "worktree"
}
