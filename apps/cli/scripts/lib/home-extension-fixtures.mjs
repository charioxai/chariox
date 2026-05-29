import { mkdir, writeFile } from "node:fs/promises"
import http from "node:http"
import path from "node:path"

export async function createHomeExtensionFixtures({
  rootDir,
  workspace,
  homeOnlyMcpPort,
  homeMarker,
  homeMcpMarker,
  homeConnectorMarker,
}) {
  await mkdir(workspace, { recursive: true })
  const homeCapabilityRoot = path.join(rootDir, "home-capabilities")
  const workerCapabilityRoot = path.join(rootDir, "worker-capabilities")
  const scriptPath = path.join(rootDir, "home_only_lookup.py")
  await writeFile(scriptPath, `
MARKER = ${JSON.stringify(homeMarker)}

def run(query: str) -> dict[str, object]:
    """Return a deterministic home-only lookup result."""
    with open(MARKER, "w", encoding="utf-8") as handle:
        handle.write("HOME_SCRIPT_EXECUTED:" + query)
    return {"query": query, "origin": "home"}

def test_run() -> None:
    result = run("self-test")
    assert result["origin"] == "home"
`, "utf8")

  const homeMcpDir = path.join(homeCapabilityRoot, "user", "mcps")
  await mkdir(homeMcpDir, { recursive: true })
  await writeFile(path.join(homeMcpDir, "home_echo_mcp.json"), `${JSON.stringify({
    name: "home_echo_mcp",
    transport: {
      type: "streamable_http",
      url: `http://127.0.0.1:${homeOnlyMcpPort}/mcp`,
    },
    enabled: true,
    required: false,
    tool_timeout_sec: 10,
  }, null, 2)}\n`, "utf8")

  const homeOnlyMcp = http.createServer(async (req, res) => {
    let body = ""
    req.setEncoding("utf8")
    for await (const chunk of req) body += chunk
    const rpc = body ? JSON.parse(body) : {}
    res.setHeader("content-type", "application/json")
    if (rpc.method === "tools/list") {
      return res.end(JSON.stringify({
        jsonrpc: "2.0",
        id: rpc.id ?? null,
        result: {
          tools: [{
            name: "home_echo",
            description: "Home-only MCP echo tool.",
            inputSchema: {
              type: "object",
              required: ["text"],
              properties: { text: { type: "string" } },
              additionalProperties: false,
            },
          }],
        },
      }))
    }
    if (rpc.method === "tools/call" && rpc.params?.name === "home_echo") {
      const text = String(rpc.params?.arguments?.text ?? "")
      await writeFile(homeMcpMarker, `HOME_MCP_EXECUTED:${text}`, "utf8")
      return res.end(JSON.stringify({
        jsonrpc: "2.0",
        id: rpc.id ?? null,
        result: {
          content: [{ type: "text", text: JSON.stringify({ origin: "home-mcp", text }) }],
        },
      }))
    }
    res.end(JSON.stringify({
      jsonrpc: "2.0",
      id: rpc.id ?? null,
      error: { code: -32601, message: `unsupported MCP method ${rpc.method}` },
    }))
  })
  await new Promise((resolve, reject) => {
    homeOnlyMcp.once("error", reject)
    homeOnlyMcp.listen(homeOnlyMcpPort, "127.0.0.1", resolve)
  })

  const connectorAdapterDir = path.join(rootDir, "home-connector-adapter")
  await mkdir(connectorAdapterDir, { recursive: true })
  const connectorAdapterScript = path.join(connectorAdapterDir, "home_connector_adapter.mjs")
  await writeFile(connectorAdapterScript, `
import { appendFileSync, writeFileSync } from 'node:fs'
import readline from 'node:readline'

const marker = process.argv[2]
const rl = readline.createInterface({ input: process.stdin })
rl.on('line', (line) => {
  const request = JSON.parse(line)
  if (request.type === 'shutdown') process.exit(0)
  if (request.type === 'validate') {
    console.log(JSON.stringify({ id: request.id, ok: true }))
    return
  }
  if (request.type === 'prepare') {
    console.log(JSON.stringify({ id: request.id, ok: true, result: { credential_targets: [], prepared_config: { arguments: request.arguments ?? {}, config: request.config ?? {} } } }))
    return
  }
  if (request.type === 'call') {
    const q = String(request.config?.arguments?.q ?? '')
    writeFileSync(marker, 'HOME_CONNECTOR_EXECUTED:' + q, 'utf8')
    console.log(JSON.stringify({ id: request.id, ok: true, result: { origin: 'home-connector', q } }))
    return
  }
  appendFileSync(marker + '.errors', 'unsupported request ' + request.type + '\\n')
  console.log(JSON.stringify({ id: request.id, ok: false, error: 'unsupported request ' + request.type }))
})
`, "utf8")

  const connectorAdapterPath = path.join(connectorAdapterDir, "adapter.yaml")
  const connectorPath = path.join(rootDir, "home-local-api-connector.yaml")
  await writeFile(connectorAdapterPath, `
kind: connector_adapter
name: home_stub
version: 0.1.0
adapter_protocol: arroba-connector-adapter-v2
command: ${process.execPath}
args:
  - ${connectorAdapterScript}
  - ${homeConnectorMarker}
description: Home-only connector adapter for remote extension drill.
`, "utf8")
  await writeFile(connectorPath, `
kind: connector
name: home_local_api
description: Home-only HTTP connector for remote extension drill.
adapter: home_stub
credential:
  required: false
timeout_ms: 30000
max_response_bytes: 1048576
operations:
  - name: public_echo
    description: Read home-only connector echo data.
    safety: read
    input_schema:
      type: object
      required: [q]
      properties:
        q: { type: string }
      additionalProperties: false
    config:
      marker: ${homeConnectorMarker}
`, "utf8")

  return {
    connectorAdapterPath,
    connectorPath,
    homeCapabilityRoot,
    homeOnlyMcp,
    scriptPath,
    workerCapabilityRoot,
  }
}
