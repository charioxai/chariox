#!/usr/bin/env node
import path from "node:path"

import { ControlledExecHarness } from "./controlled-exec-harness.mjs"
import { InteractionGateway } from "./interaction-gateway.mjs"
import { JsonRpcFramer, writeJsonRpc } from "../../mcp-isolation-spike/src/mcp-framing.mjs"

function argValue(name, fallback = null) {
  const index = process.argv.indexOf(name)
  if (index < 0) return fallback
  return process.argv[index + 1] ?? fallback
}

const cwd = path.resolve(argValue("--cwd", process.cwd()))
const scriptedResponses = JSON.parse(argValue("--responses", '["yes", {"choiceId":"beta"}, "yes"]'))
const responseDelayMs = Number.parseInt(argValue("--response-delay-ms", "0"), 10) || 0
const interactionGateway = new InteractionGateway({
  async onRequest(interaction) {
    if (responseDelayMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, responseDelayMs))
    }
    if (interaction.kind === "choice") {
      return {
        choiceId: "beta",
        reply: "User picked beta.",
      }
    }
    return { choiceId: "yes", reply: "User approved." }
  },
  responses: scriptedResponses,
})
const harness = new ControlledExecHarness({ interactions: interactionGateway })

const protocolVersion = "2024-11-05"
let responseFrameFormat = "line"

function reply(message) {
  writeJsonRpc(process.stdout, message, responseFrameFormat)
}

async function handle(message) {
  if (!message || typeof message !== "object") return
  const { id, method, params } = message
  try {
    if (method === "initialize") {
      reply({
        jsonrpc: "2.0",
        id,
        result: {
          protocolVersion,
          capabilities: { tools: {} },
          serverInfo: { name: "chariox-controlled-exec-spike", version: "0.0.0-spike" },
        },
      })
      return
    }
    if (method === "notifications/initialized") return
    if (method === "tools/list") {
      reply({
        jsonrpc: "2.0",
        id,
        result: {
          tools: [
            {
              name: "controlled_exec",
              description: "Execute a command through a Chariox-like permission gate.",
              inputSchema: {
                type: "object",
                properties: {
                  agent_id: { type: "string" },
                  turn_id: { type: "string" },
                  command: { type: "string" },
                  permission_mode: { type: "string", enum: ["limited", "yolo", "yolo+rm"] },
                },
                required: ["command"],
              },
            },
            {
              name: "request_popup",
              description: "Block on a scripted popup response to simulate a synchronous Chariox interaction.",
              inputSchema: {
                type: "object",
                properties: {
                  agent_id: { type: "string" },
                  turn_id: { type: "string" },
                  title: { type: "string" },
                  message: { type: "string" },
                  level: { type: "string", enum: ["info", "warning", "critical"] },
                  choices: {
                    type: "array",
                    items: {
                      anyOf: [
                        { type: "string" },
                        {
                          type: "object",
                          properties: {
                            id: { type: "string" },
                            label: { type: "string" },
                            reply: { type: "string" },
                          },
                          required: ["id", "label"],
                        },
                      ],
                    },
                  },
                  default_on_timeout: { type: "string" },
                  timeout_sec: { type: "number" },
                },
                required: ["title", "message", "choices"],
              },
            },
          ],
        },
      })
      return
    }
    if (method === "tools/call") {
      if (params?.name === "controlled_exec") {
        const result = await harness.execute({
          agentId: params.arguments?.agent_id ?? "provider-agent",
          turnId: params.arguments?.turn_id ?? null,
          command: params.arguments?.command ?? "",
          permissionMode: params.arguments?.permission_mode ?? "limited",
          cwd,
        })
        reply({
          jsonrpc: "2.0",
          id,
          result: {
            content: [
              { type: "text", text: JSON.stringify(result) },
            ],
          },
        })
        return
      }
      if (params?.name === "request_popup") {
        const result = await interactionGateway.request({
          agentId: params.arguments?.agent_id ?? "provider-agent",
          turnId: params.arguments?.turn_id ?? null,
          kind: "choice",
          severity: params.arguments?.level ?? "info",
          title: params.arguments?.title ?? "Pick one",
          message: params.arguments?.message ?? "",
          choices: (params.arguments?.choices ?? []).map((choice) => typeof choice === "string"
            ? { id: choice, label: choice, reply: choice }
            : {
                id: choice.id,
                label: choice.label,
                reply: choice.reply ?? null,
              }),
          defaultChoice: params.arguments?.default_on_timeout ?? null,
          timeoutSec: params.arguments?.timeout_sec ?? null,
        })
        reply({
          jsonrpc: "2.0",
          id,
          result: {
            content: [
              { type: "text", text: JSON.stringify(result) },
            ],
          },
        })
        return
      }
      reply({ jsonrpc: "2.0", id, error: { code: -32601, message: `unknown tool ${params?.name}` } })
      return
    }
    reply({ jsonrpc: "2.0", id, error: { code: -32601, message: `unknown method ${method}` } })
  } catch (error) {
    reply({ jsonrpc: "2.0", id, error: { code: -32000, message: error.message } })
  }
}

new JsonRpcFramer(
  process.stdin,
  (message) => { void handle(message) },
  { onFrame: (format) => { responseFrameFormat = format } },
)
process.stdin.resume()
