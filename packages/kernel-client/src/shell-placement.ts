export type PlacementOptions = {
  positional: string[]
  directory?: string | undefined
  gitWorktree?: string | undefined
  branch?: string | undefined
  fromRef?: string | undefined
  machineRef?: string | undefined
  kernelRef?: string | undefined
  sliceRef?: string | undefined
  sliceDisplayMode?: "headless" | "headed" | undefined
}

export type ShellPlacementDeps = {
}

export type GitWorktreePlacementInput = {
  target_directory?: string | null
  branch?: string | null
  from_ref?: string | null
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
    } else if ((arg === "--machine" || arg === "--kernel") && next && allowMachine) {
      if (arg === "--machine") {
        options.machineRef = next
      } else {
        options.kernelRef = next
      }
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
  if (options.machineRef && options.kernelRef) {
    return { options, error: "use either --machine or --kernel, not both" }
  }
  const sliceCreatesPlacement = options.sliceRef === "new" || options.sliceRef === "new:headless" || options.sliceRef === "new:headed"
  if ((options.machineRef || options.kernelRef) && options.sliceRef && !sliceCreatesPlacement) {
    return { options, error: "use either --machine/--kernel or a reusable --slice, not both" }
  }
  if (options.sliceDisplayMode && !sliceCreatesPlacement) {
    return { options, error: "--slice-display requires --slice new" }
  }
  return { options }
}

export async function resolveShellPlacement(
  options: PlacementOptions,
  _baseDirectory: string,
  _label: string,
  _deps: ShellPlacementDeps,
  allowPositionalDirectory = true,
): Promise<string | undefined> {
  const positionalDirectory = allowPositionalDirectory && options.positional.length === 1 && !options.directory && !options.gitWorktree
    ? options.positional[0]
    : undefined
  if (positionalDirectory || options.directory) {
    return resolvePlacementPath(_baseDirectory, positionalDirectory ?? options.directory!)
  }
  return undefined
}

export function shellGitWorktreePlacement(options: PlacementOptions): GitWorktreePlacementInput | undefined {
  if (!options.gitWorktree && !options.branch && !options.fromRef) {
    return undefined
  }
  return {
    target_directory: options.gitWorktree ?? null,
    branch: options.branch ?? null,
    from_ref: options.fromRef ?? null,
  }
}

function resolvePlacementPath(baseDirectory: string, directory: string): string {
  if (directory.startsWith("/")) {
    return normalizePath(directory)
  }
  return normalizePath(`${baseDirectory.replace(/\/+$/, "")}/${directory}`)
}

function normalizePath(path: string): string {
  const absolute = path.startsWith("/")
  const parts: string[] = []
  for (const part of path.split("/")) {
    if (!part || part === ".") {
      continue
    }
    if (part === "..") {
      if (parts.length > 0) {
        parts.pop()
      }
      continue
    }
    parts.push(part)
  }
  return `${absolute ? "/" : ""}${parts.join("/")}` || (absolute ? "/" : ".")
}
