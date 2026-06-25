#!/usr/bin/env node
import { spawn } from "node:child_process"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import http from "node:http"
import os from "node:os"
import path from "node:path"
import { setTimeout as sleep } from "node:timers/promises"

const defaultTimeoutMs = 15_000

function parseArgs(argv) {
  const options = {
    timeoutMs: defaultTimeoutMs,
    keepArtifacts: false,
  }
  for (let index = 2; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--timeout-ms") {
      options.timeoutMs = Number(argv[++index] ?? defaultTimeoutMs)
    } else if (arg === "--keep-artifacts-on-failure") {
      options.keepArtifacts = true
    } else if (arg === "--help" || arg === "-h") {
      printHelp()
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) {
    throw new Error("--timeout-ms must be a positive number")
  }
  return options
}

function printHelp() {
  console.log(`Usage: node apps/cli/scripts/live-pi-rpc-extension-smoke-drill.mjs [options]

Starts real "pi --mode rpc" with a temporary extension and a fake Arroba-style
MCP HTTP endpoint. The drill does not submit an LLM prompt and does not require
backing-provider credentials.

Options:
  --timeout-ms 15000
  --keep-artifacts-on-failure
`)
}

function extensionSource() {
  return `import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

type McpServer = { name: string; url: string; headers?: Record<string, string> };
let server: McpServer;

async function callMcp(method: string, params?: unknown): Promise<any> {
  const response = await fetch(server.url, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      ...(server.headers ?? {}),
    },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: Date.now(),
      method,
      params: params ?? {},
    }),
  });
  if (!response.ok) {
    throw new Error(\`MCP \${method} failed with HTTP \${response.status}: \${await response.text()}\`);
  }
  const payload = await response.json();
  if (payload.error) {
    throw new Error(payload.error.message ?? JSON.stringify(payload.error));
  }
  return payload.result;
}

export default async function(pi: ExtensionAPI) {
  server = JSON.parse(process.env.ARROBA_PI_MCP_SERVERS ?? "[]")[0] as McpServer;
  if (!server) throw new Error("ARROBA_PI_MCP_SERVERS did not include a server");
  await callMcp("initialize", {
    protocolVersion: "2025-03-26",
    capabilities: {},
    clientInfo: { name: "arroba-pi-rpc-extension-smoke", version: "1" },
  }).catch(() => {});
  const listed = await callMcp("tools/list");
  if (!listed.tools?.some((tool: any) => tool.name === "ping")) {
    throw new Error("fake MCP ping tool was not listed");
  }
  pi.registerCommand("arroba-pi-mcp-smoke", {
    description: "Run Arroba Pi MCP smoke",
    handler: async () => {
      const result = await callMcp("tools/call", {
        name: "ping",
        arguments: { value: "ok" },
      });
      if (result?.structuredContent?.ok !== true) {
        throw new Error(\`unexpected fake MCP result: \${JSON.stringify(result)}\`);
      }
    },
  });
}
`
}

function createFakeMcpServer() {
  const seen = []
  const server = http.createServer((request, response) => {
    let body = ""
    request.on("data", (chunk) => {
      body += chunk.toString()
    })
    request.on("end", () => {
      const payload = body ? JSON.parse(body) : {}
      seen.push(payload.method)
      let result = {}
      if (payload.method === "initialize") {
        result = {
          protocolVersion: "2025-03-26",
          capabilities: { tools: { listChanged: false } },
          serverInfo: { name: "fake-arroba-mcp", version: "1" },
        }
      } else if (payload.method === "tools/list") {
        result = {
          tools: [{
            name: "ping",
            description: "Fake ping tool",
            inputSchema: {
              type: "object",
              properties: { value: { type: "string" } },
              additionalProperties: false,
            },
          }],
        }
      } else if (payload.method === "tools/call") {
        result = {
          content: [{ type: "text", text: "pong" }],
          structuredContent: { ok: true, arguments: payload.params?.arguments ?? {} },
          isError: false,
        }
      } else {
        response.writeHead(200, { "content-type": "application/json" })
        response.end(JSON.stringify({
          jsonrpc: "2.0",
          id: payload.id,
          error: { code: -32601, message: `method not found: ${payload.method}` },
        }))
        return
      }
      response.writeHead(200, { "content-type": "application/json" })
      response.end(JSON.stringify({ jsonrpc: "2.0", id: payload.id, result }))
    })
  })
  return { server, seen }
}

async function listen(server) {
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, "127.0.0.1", resolve)
  })
  const address = server.address()
  return `http://127.0.0.1:${address.port}/mcp`
}

async function closeServer(server) {
  await new Promise((resolve) => server.close(resolve))
}

function spawnPi({ extensionPath, mcpUrl }) {
  return spawn("pi", [
    "--mode", "rpc",
    "--no-session",
    "--offline",
    "--no-context-files",
    "--no-skills",
    "--no-prompt-templates",
    "--no-themes",
    "--provider", "openai-codex",
    "--model", "openai-codex/gpt-5.4",
    "--extension", extensionPath,
  ], {
    stdio: ["pipe", "pipe", "pipe"],
    env: {
      ...process.env,
      ARROBA_PI_MCP_SERVERS: JSON.stringify([{ name: "fake", url: mcpUrl }]),
    },
  })
}

async function stopChild(child) {
  if (!child || child.exitCode != null) return
  child.kill("SIGTERM")
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(2_000)])
  if (child.exitCode == null) {
    child.kill("SIGKILL")
    await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(1_000)])
  }
}

function send(child, payload) {
  child.stdin.write(`${JSON.stringify(payload)}\n`)
}

async function waitFor({ responses, id, child, timeoutMs }) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const response = responses.find((entry) => entry.id === id)
    if (response) return response
    if (child.exitCode != null) throw new Error(`pi exited before ${id}: ${child.exitCode}`)
    await sleep(50)
  }
  throw new Error(`timed out waiting for ${id}`)
}

async function main() {
  const options = parseArgs(process.argv)
  const tmpRoot = await mkdtemp(path.join(os.tmpdir(), "arroba-pi-rpc-extension-smoke-"))
  const extensionPath = path.join(tmpRoot, "arroba-pi-rpc-extension-smoke.ts")
  const { server, seen } = createFakeMcpServer()
  let piProcess
  try {
    await writeFile(extensionPath, extensionSource())
    const mcpUrl = await listen(server)
    piProcess = spawnPi({ extensionPath, mcpUrl })
    const responses = []
    let stderr = ""
    piProcess.stdout.on("data", (chunk) => {
      for (const line of chunk.toString().split("\n")) {
        if (!line.trim()) continue
        responses.push(JSON.parse(line))
      }
    })
    piProcess.stderr.on("data", (chunk) => {
      stderr += chunk.toString()
    })

    send(piProcess, { id: "state", type: "get_state" })
    const state = await waitFor({ responses, id: "state", child: piProcess, timeoutMs: options.timeoutMs })
    if (state.success !== true || state.data?.sessionId == null) {
      throw new Error(`Pi get_state did not return a session id: ${JSON.stringify(state)}`)
    }

    send(piProcess, { id: "commands", type: "get_commands" })
    const commands = await waitFor({ responses, id: "commands", child: piProcess, timeoutMs: options.timeoutMs })
    const commandNames = commands.data?.commands?.map((command) => command.name) ?? []
    if (!commandNames.includes("arroba-pi-mcp-smoke")) {
      throw new Error(`Pi extension command was not registered: ${JSON.stringify(commands)}`)
    }

    send(piProcess, { id: "prompt", type: "prompt", message: "/arroba-pi-mcp-smoke" })
    const prompt = await waitFor({ responses, id: "prompt", child: piProcess, timeoutMs: options.timeoutMs })
    if (prompt.success !== true) {
      throw new Error(`Pi extension command prompt failed: ${JSON.stringify(prompt)}`)
    }
    await sleep(250)
    if (!seen.includes("tools/call")) {
      throw new Error(`fake MCP tools/call was not observed; saw ${seen.join(",")}`)
    }
    if (stderr.trim()) {
      throw new Error(`Pi wrote unexpected stderr:\n${stderr}`)
    }
    console.log("pi RPC extension smoke drill passed")
  } catch (error) {
    if (options.keepArtifacts) {
      console.error(`kept artifacts in ${tmpRoot}`)
    }
    throw error
  } finally {
    await stopChild(piProcess)
    await closeServer(server).catch(() => {})
    if (!options.keepArtifacts) await rm(tmpRoot, { recursive: true, force: true })
  }
}

main().catch((error) => {
  console.error(error?.stack ?? error?.message ?? String(error))
  process.exit(1)
})
