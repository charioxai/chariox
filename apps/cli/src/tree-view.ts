import path from "node:path"

export type DirectoryTreeEntry = {
  relative_path: string
  kind: string
}

export type DirectoryTreeState = {
  rootPath: string
  entries: DirectoryTreeEntry[]
  expandedPaths: string[]
  loadedPaths: string[]
  selectedPath: string
}

export type DirectoryTreeRow = {
  id: string
  depth: number
  label: string
  kind: "root" | "directory" | "file" | "symlink"
  expanded?: boolean
}

type TreeNode = {
  id: string
  name: string
  kind: "root" | "directory" | "file" | "symlink"
  children: Map<string, TreeNode>
}

export function createDirectoryTreeState(rootPath: string, entries: DirectoryTreeEntry[]): DirectoryTreeState {
  return {
    rootPath,
    entries,
    expandedPaths: [""],
    loadedPaths: [""],
    selectedPath: "",
  }
}

export function mergeDirectoryTreeEntries(
  state: DirectoryTreeState,
  basePath: string,
  entries: DirectoryTreeEntry[],
) {
  const prefix = basePath ? `${basePath}/` : ""
  const merged = new Map(state.entries.map((entry) => [entry.relative_path, entry]))
  for (const entry of entries) {
    merged.set(`${prefix}${entry.relative_path}`.replace(/^\//, ""), {
      ...entry,
      relative_path: `${prefix}${entry.relative_path}`.replace(/^\//, ""),
    })
  }
  const loadedPaths = new Set(state.loadedPaths)
  loadedPaths.add(basePath)
  return {
    ...state,
    entries: [...merged.values()],
    loadedPaths: [...loadedPaths],
  }
}

export function isDirectoryTreePathLoaded(state: DirectoryTreeState, path: string) {
  return state.loadedPaths.includes(path)
}

export function buildDirectoryTreeRows(state: DirectoryTreeState): DirectoryTreeRow[] {
  const root = buildTree(state.rootPath, state.entries)
  const expanded = new Set(state.expandedPaths)
  const rows: DirectoryTreeRow[] = []

  const visit = (node: TreeNode, depth: number) => {
    const row: DirectoryTreeRow = {
      id: node.id,
      depth,
      label: node.kind === "root" ? path.basename(state.rootPath) || state.rootPath : node.name,
      kind: node.kind,
    }
    if (node.kind === "root" || node.kind === "directory") {
      row.expanded = expanded.has(node.id)
    }
    rows.push(row)
    if ((node.kind === "root" || node.kind === "directory") && expanded.has(node.id)) {
      for (const child of [...node.children.values()].sort(compareTreeNodes)) {
        visit(child, depth + 1)
      }
    }
  }

  visit(root, 0)
  return rows
}

export function moveDirectoryTreeSelection(state: DirectoryTreeState, direction: "up" | "down") {
  const rows = buildDirectoryTreeRows(state)
  const currentIndex = Math.max(0, rows.findIndex((row) => row.id === state.selectedPath))
  const nextIndex = direction === "up"
    ? Math.max(0, currentIndex - 1)
    : Math.min(rows.length - 1, currentIndex + 1)
  return {
    ...state,
    selectedPath: rows[nextIndex]?.id ?? state.selectedPath,
  }
}

export function toggleDirectoryTreeExpansion(state: DirectoryTreeState) {
  const rows = buildDirectoryTreeRows(state)
  const selected = rows.find((row) => row.id === state.selectedPath)
  if (!selected || (selected.kind !== "root" && selected.kind !== "directory")) {
    return state
  }

  const expanded = new Set(state.expandedPaths)
  if (expanded.has(selected.id)) {
    expanded.delete(selected.id)
  } else {
    expanded.add(selected.id)
  }
  expanded.add("")
  return {
    ...state,
    expandedPaths: [...expanded],
  }
}

function buildTree(rootPath: string, entries: DirectoryTreeEntry[]) {
  const root: TreeNode = {
    id: "",
    name: path.basename(rootPath) || rootPath,
    kind: "root",
    children: new Map(),
  }

  for (const entry of entries) {
    const segments = entry.relative_path.split("/").filter(Boolean)
    let current = root
    for (let index = 0; index < segments.length; index += 1) {
      const name = segments[index]!
      const id = segments.slice(0, index + 1).join("/")
      const existing = current.children.get(name)
      const kind = index === segments.length - 1
        ? normalizeKind(entry.kind)
        : "directory"
      const next = existing ?? {
        id,
        name,
        kind,
        children: new Map(),
      }
      if (!existing) {
        current.children.set(name, next)
      }
      current = next
    }
  }

  return root
}

function compareTreeNodes(left: TreeNode, right: TreeNode) {
  const leftDir = left.kind === "directory"
  const rightDir = right.kind === "directory"
  if (leftDir !== rightDir) {
    return leftDir ? -1 : 1
  }
  return left.name.localeCompare(right.name)
}

function normalizeKind(kind: string): TreeNode["kind"] {
  if (kind === "directory") return "directory"
  if (kind === "symlink") return "symlink"
  return "file"
}
