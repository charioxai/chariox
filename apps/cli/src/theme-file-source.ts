import os from "node:os"
import path from "node:path"
import { readdir } from "node:fs/promises"

export type ThemeLoadWarning = {
  path: string
  message: string
}

export type ThemeLoadOptions = {
  workspace?: string | null
  directories?: string[]
  onWarning?: (warning: ThemeLoadWarning) => void
}

export function themeDirectories(workspace?: string | null) {
  const directories = [path.join(configRoot(), "themes")]
  if (workspace?.trim()) {
    directories.push(path.join(workspace, ".chariox", "themes"))
  }
  return directories
}

export async function themeFiles(directory: string) {
  try {
    return (await readdir(directory, { withFileTypes: true }))
      .filter((entry) => entry.isFile() && entry.name.endsWith(".json"))
      .map((entry) => path.join(directory, entry.name))
      .sort((a, b) => a.localeCompare(b))
  } catch {
    return []
  }
}

function configRoot() {
  const xdg = process.env.XDG_CONFIG_HOME?.trim()
  if (xdg) {
    return path.join(xdg, "chariox")
  }
  return path.join(os.homedir(), ".chariox")
}
