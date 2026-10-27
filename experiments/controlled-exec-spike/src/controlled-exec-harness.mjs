import { spawn } from "node:child_process"
import { access, mkdir } from "node:fs/promises"
import path from "node:path"

import { InteractionGateway } from "./interaction-gateway.mjs"
import { PermissionLedger } from "./permission-ledger.mjs"

function shellSplit(command) {
  return command.match(/"[^"]*"|'[^']*'|[^\s]+/g) ?? []
}

function stripQuotes(value) {
  if (!value) return value
  if ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith("'") && value.endsWith("'"))) {
    return value.slice(1, -1)
  }
  return value
}

async function pathExists(targetPath) {
  try {
    await access(targetPath)
    return true
  } catch {
    return false
  }
}

function isFlag(token) {
  return token.startsWith("-")
}

function resolveTargets(tokens, cwd) {
  return tokens.filter(Boolean).map((token) => path.resolve(cwd, stripQuotes(token)))
}

export function classifyCommand(command, cwd) {
  const tokens = shellSplit(command)
  const executable = stripQuotes(tokens[0] ?? "")
  const redirects = [...command.matchAll(/(?:^|[^>])>(?:>|)\s*([^\s]+)/g)].map((match) => path.resolve(cwd, stripQuotes(match[1])))
  const classification = {
    executable,
    tokens,
    creates: [],
    writes: [],
    deletes: [],
    deleteFlags: [],
    destructive: false,
  }

  if (redirects.length > 0) {
    classification.writes.push(...redirects)
  }

  if (executable === "mkdir") {
    const targets = resolveTargets(tokens.slice(1).filter((token) => !isFlag(token)), cwd)
    classification.creates.push(...targets)
  } else if (executable === "touch") {
    classification.creates.push(...resolveTargets(tokens.slice(1), cwd))
  } else if (executable === "rm") {
    const flags = tokens.slice(1).filter(isFlag)
    classification.deleteFlags = flags
    classification.deletes.push(...resolveTargets(tokens.slice(1).filter((token) => !isFlag(token)), cwd))
    classification.destructive = classification.deletes.length > 0
  } else if (executable === "mv") {
    const parts = tokens.slice(1).filter((token) => !isFlag(token))
    if (parts.length >= 2) {
      classification.creates.push(path.resolve(cwd, stripQuotes(parts[parts.length - 1])))
    }
  }

  return classification
}

async function runCommand(command, cwd, env = {}) {
  await mkdir(cwd, { recursive: true })
  return await new Promise((resolve, reject) => {
    const child = spawn(process.env.SHELL || "zsh", ["-lc", command], {
      cwd,
      env: { ...process.env, ...env },
      stdio: ["ignore", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", (chunk) => {
      stdout += String(chunk)
    })
    child.stderr.on("data", (chunk) => {
      stderr += String(chunk)
    })
    child.once("error", reject)
    child.once("close", (exitCode, signal) => {
      resolve({ stdout, stderr, exitCode: exitCode ?? 1, signal })
    })
  })
}

export class ControlledExecHarness {
  constructor(options = {}) {
    this.options = options
    this.interactions = options.interactions ?? new InteractionGateway()
    this.ledger = options.ledger ?? new PermissionLedger()
  }

  async execute({
    agentId,
    command,
    cwd = process.cwd(),
    permissionMode = "limited",
    turnId = null,
    env = {},
  }) {
    const classification = classifyCommand(command, cwd)
    const permission = await this.#authorize({
      agentId,
      cwd,
      permissionMode,
      turnId,
      command,
      classification,
    })
    if (!permission.allowed) {
      return {
        ok: false,
        allowed: false,
        permissionMode,
        command,
        cwd,
        classification,
        approval: permission.approval ?? null,
        deniedReason: permission.reason,
        stdout: "",
        stderr: "",
        exitCode: null,
      }
    }

    const candidateCreates = [...new Set([...classification.creates, ...classification.writes])]
    const preExisting = new Map()
    for (const targetPath of candidateCreates) {
      preExisting.set(targetPath, await pathExists(targetPath))
    }

    const result = await runCommand(command, cwd, env)

    if (result.exitCode === 0) {
      for (const targetPath of candidateCreates) {
        if (!preExisting.get(targetPath) && await pathExists(targetPath)) {
          this.ledger.recordCreatedPath(targetPath, agentId)
        }
      }
      for (const targetPath of classification.deletes) {
        if (!await pathExists(targetPath)) {
          this.ledger.removePath(targetPath)
        }
      }
    }

    return {
      ok: result.exitCode === 0,
      allowed: true,
      permissionMode,
      command,
      cwd,
      classification,
      approval: permission.approval ?? null,
      stdout: result.stdout,
      stderr: result.stderr,
      exitCode: result.exitCode,
      signal: result.signal,
    }
  }

  async #authorize({ agentId, cwd, permissionMode, turnId, command, classification }) {
    if (permissionMode === "yolo+rm") {
      return { allowed: true, approval: { choiceId: "auto" } }
    }

    if (permissionMode === "yolo") {
      if (classification.deletes.length === 0) {
        return { allowed: true, approval: { choiceId: "auto" } }
      }
      for (const targetPath of classification.deletes) {
        const deleteCheck = await this.ledger.canDeletePath(targetPath, agentId)
        if (!deleteCheck.allowed) {
          return {
            allowed: false,
            reason: `delete blocked in yolo: ${deleteCheck.offendingPath ?? targetPath} is not owned by ${agentId}`,
          }
        }
      }
      return { allowed: true, approval: { choiceId: "auto" } }
    }

    const choice = await this.interactions.request({
      turnId,
      agentId,
      kind: "permission",
      severity: classification.destructive ? "critical" : "warning",
      title: "Approve command",
      message: command,
      choices: [
        { id: "yes", label: "Yes" },
        { id: "no", label: "No" },
      ],
      defaultChoice: "no",
    })
    if (choice.choiceId !== "yes") {
      return {
        allowed: false,
        approval: choice,
        reason: `command denied by user: ${command}`,
      }
    }
    return { allowed: true, approval: choice }
  }
}
