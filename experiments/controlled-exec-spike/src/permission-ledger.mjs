import { readdir, stat } from "node:fs/promises"
import path from "node:path"

export class PermissionLedger {
  constructor() {
    this.createdPaths = new Map()
  }

  recordCreatedPath(targetPath, agentId) {
    const normalized = path.resolve(targetPath)
    this.createdPaths.set(normalized, { agentId, createdAtMs: Date.now() })
  }

  ownerForPath(targetPath) {
    return this.createdPaths.get(path.resolve(targetPath)) ?? null
  }

  removePath(targetPath) {
    const normalized = path.resolve(targetPath)
    for (const key of [...this.createdPaths.keys()]) {
      if (key === normalized || key.startsWith(`${normalized}${path.sep}`)) {
        this.createdPaths.delete(key)
      }
    }
  }

  async canDeletePath(targetPath, agentId) {
    const normalized = path.resolve(targetPath)
    return await this.#canDeleteRecursive(normalized, agentId)
  }

  async #canDeleteRecursive(targetPath, agentId) {
    let targetStat
    try {
      targetStat = await stat(targetPath)
    } catch (error) {
      if (error?.code === "ENOENT") return { allowed: true, missing: true, checkedPaths: [] }
      throw error
    }
    const checkedPaths = [targetPath]
    const owner = this.ownerForPath(targetPath)
    if (!owner || owner.agentId !== agentId) {
      return { allowed: false, offendingPath: targetPath, checkedPaths }
    }
    if (!targetStat.isDirectory()) {
      return { allowed: true, checkedPaths }
    }
    const entries = await readdir(targetPath)
    for (const entry of entries) {
      const childPath = path.join(targetPath, entry)
      const child = await this.#canDeleteRecursive(childPath, agentId)
      checkedPaths.push(...(child.checkedPaths ?? []))
      if (!child.allowed) return child
    }
    return { allowed: true, checkedPaths }
  }
}
