import { setTimeout as sleep } from "node:timers/promises"

export async function callRuntimeMcp(serverUrl, authToken, method, params = {}) {
  const response = await fetch(serverUrl, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${authToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ jsonrpc: "2.0", id: `${Date.now()}`, method, params }),
  })
  const json = await response.json()
  if (json.error) throw new Error(`runtime MCP ${method} failed: ${JSON.stringify(json.error)}`)
  return json.result
}

export async function expectRuntimeMcpReject(serverUrl, authToken, method, params = {}) {
  let result
  try {
    result = await callRuntimeMcp(serverUrl, authToken, method, params)
  } catch (error) {
    return { error: String(error?.message ?? error) }
  }
  if (result?.isError) return result
  throw new Error(`runtime MCP ${method} unexpectedly succeeded: ${JSON.stringify(result)}`)
}

export async function expectReject(label, fn, expectedText) {
  try {
    await fn()
  } catch (error) {
    const message = String(error?.message ?? error)
    if (!message.includes(expectedText)) {
      throw new Error(`${label} rejected with unexpected error. Expected ${expectedText}, got: ${message}`)
    }
    return message
  }
  throw new Error(`${label} unexpectedly succeeded`)
}

export async function waitForRuntimeTool(serverUrl, authToken, name, present) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const tools = await callRuntimeMcp(serverUrl, authToken, "tools/list")
    const found = tools.tools.some((tool) => tool.name === name)
    if (found === present) return tools
    await sleep(250)
  }
  throw new Error(`tool ${name} did not become ${present ? "advertised" : "revoked"}`)
}
