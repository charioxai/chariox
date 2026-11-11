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

/** A tool's name without separators or case. The Chariox bridge's tools arrive as "chariox.x", "chariox_x" or, through a provider's MCP client,
 *  "mcp__chariox__chariox_x" or "mcp__chariox__x": one name each ("charioxx"). */
export function compactToolName(tool: unknown): string {
  return typeof tool === "string" ? tool.trim().replace(/[\s._-]/g, "").toLowerCase().replace(/^mcp(?=chariox)/, "").replace(/^(?:chariox)+(?=chariox)/, "") : ""
}

export function canonicalToolName(tool: unknown) {
  if (typeof tool !== "string") {
    return ""
  }
  const normalized = tool.trim()
  const compact = compactToolName(normalized)
  const chariox = compact.startsWith("chariox")
  const verb = compact.replace(/^(?:chariox)+/, "")
  if (verb === "writeartifact") return "chariox.write_artifact"
  if (verb === "editartifact") return "chariox.edit_artifact"
  if (verb === "patchartifact" || (chariox && verb === "applypatch")) return "chariox.apply_patch"
  if (verb === "applypatch") return "apply_patch"
  if (verb === "deleteartifact") return "chariox.delete_artifact"
  if (verb === "moveartifact") return "chariox.move_artifact"
  if (verb === "readartifact") return "chariox.read_artifact"
  return normalized
}
