export function nativeToolDisplayTitle(tool: string) {
  const canonical = canonicalToolName(tool)
  if (canonical === "chariox.read_artifact") return "read"
  if (isWorkspaceLiveSyncTool(tool)) return "patch"
  return tool
}

export function isNativeReadTool(tool: unknown) {
  const canonical = canonicalToolName(tool)
  return canonical === "read" || canonical === "chariox.read_artifact"
}

export function isWorkspaceLiveSyncTool(tool: unknown) {
  const canonical = canonicalToolName(tool)
  return canonical === "chariox.edit_artifact"
    || canonical === "chariox.apply_patch"
    || canonical === "chariox.delete_artifact"
    || canonical === "chariox.move_artifact"
    || canonical === "chariox.write_artifact"
}

function canonicalToolName(tool: unknown) {
  if (typeof tool !== "string") {
    return ""
  }
  const normalized = tool.trim()
  const compact = normalized.replace(/[._-]/g, "").toLowerCase()
  if (compact === "charioxwriteartifact" || compact === "writeartifact") return "chariox.write_artifact"
  if (compact === "charioxeditartifact" || compact === "editartifact") return "chariox.edit_artifact"
  if (
    compact === "charioxapplypatch"
    || compact === "charioxpatchartifact"
    || compact === "mcpcharioxpatchartifact"
    || compact === "mcpcharioxcharioxpatchartifact"
  ) return "chariox.apply_patch"
  if (compact === "patchartifact") return "chariox.apply_patch"
  if (compact === "applypatch") return "apply_patch"
  if (compact === "charioxdeleteartifact" || compact === "deleteartifact") return "chariox.delete_artifact"
  if (compact === "charioxmoveartifact" || compact === "moveartifact") return "chariox.move_artifact"
  if (compact === "charioxreadartifact" || compact === "readartifact") return "chariox.read_artifact"
  return normalized
}
