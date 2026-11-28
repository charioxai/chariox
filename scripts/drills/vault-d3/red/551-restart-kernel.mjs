#!/usr/bin/env node
// Row 551, static contract (no kernel needed): the owner and the agent can restart a kernel (hot reload) per machine.
// RED on main: no `kernels restart` command in the CLI command table, no restart/hot-reload request in the kernel protocol,
// no agent tool. Reads the checkout this file is in (override with D3_REPO). The Settings > Machines button is Cloud (cell in the PR body).
import { readFileSync, readdirSync, statSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { reporter } from "./host-harness.mjs"

const repo = process.env.D3_REPO ?? path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..")
const r = reporter("551")
const table = readFileSync(path.join(repo, "apps/cli/src/agent-command-table.ts"), "utf8")
r.check("CLI command table lists `kernels restart`", /name:\s*"kernels restart"/.test(table))
const walk = dir => readdirSync(dir).flatMap(n => { const p = path.join(dir, n); return statSync(p).isDirectory() ? walk(p) : [p] })
const protocolFiles = walk(path.join(repo, "apps/kernel/src/local/api/types")).filter(f => f.endsWith(".rs"))
const requestText = protocolFiles.map(f => readFileSync(f, "utf8")).join("\n")
r.check("kernel protocol has an owner request to restart/hot-reload a kernel", /\b(RestartKernel|HotReloadKernel|ReloadKernel)\b/.test(requestText))
const agentTools = walk(path.join(repo, "apps/kernel/src/transport/runtime_tools")).filter(f => true && f.endsWith(".rs")).map(f => readFileSync(f, "utf8")).join("\n")
r.check("agents have a restart-kernel tool (parity)", /chariox\.kernels\.restart|kernels_restart/.test(agentTools))
r.done()
