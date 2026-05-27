export function nativeToolDisplayTitle(tool: string) {
  const canonical = canonicalToolName(tool)
  if (canonical === "arroba.read_artifact") return "read"
  if (isWorkspaceLiveSyncTool(tool)) return "patch"
  return tool
}

export function isNativeReadTool(tool: unknown) {
  const canonical = canonicalToolName(tool)
  return canonical === "read" || canonical === "arroba.read_artifact"
}

export function isWorkspaceLiveSyncTool(tool: unknown) {
  const canonical = canonicalToolName(tool)
  return canonical === "arroba.edit_artifact"
    || canonical === "arroba.apply_patch"
    || canonical === "arroba.delete_artifact"
    || canonical === "arroba.move_artifact"
    || canonical === "arroba.write_artifact"
}

function canonicalToolName(tool: unknown) {
  if (typeof tool !== "string") {
    return ""
  }
  const normalized = tool.trim()
  const compact = normalized.replace(/[._-]/g, "").toLowerCase()
  if (compact === "arrobawriteartifact" || compact === "writeartifact") return "arroba.write_artifact"
  if (compact === "arrobaeditartifact" || compact === "editartifact") return "arroba.edit_artifact"
  if (
    compact === "arrobaapplypatch"
    || compact === "arrobapatchartifact"
    || compact === "mcparrobapatchartifact"
    || compact === "mcparrobaarrobapatchartifact"
  ) return "arroba.apply_patch"
  if (compact === "patchartifact") return "arroba.apply_patch"
  if (compact === "applypatch") return "apply_patch"
  if (compact === "arrobadeleteartifact" || compact === "deleteartifact") return "arroba.delete_artifact"
  if (compact === "arrobamoveartifact" || compact === "moveartifact") return "arroba.move_artifact"
  if (compact === "arrobareadartifact" || compact === "readartifact") return "arroba.read_artifact"
  return normalized
}
