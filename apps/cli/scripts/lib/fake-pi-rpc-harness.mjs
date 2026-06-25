import { chmod, writeFile } from "node:fs/promises"

export async function writeFakePiRpcHarness(filePath) {
  await writeFile(filePath, fakePiRpcHarnessSource(), "utf8")
  await chmod(filePath, 0o755)
}

export function fakePiRpcHarnessSource() {
  return `#!/usr/bin/env node
import { appendFileSync } from "node:fs";
import readline from "node:readline";

const logPath = process.env.ARROBA_FAKE_PI_LOG;
function log(event) {
  if (!logPath) return;
  appendFileSync(logPath, JSON.stringify({ ...event, at: Date.now() }) + "\\n");
}

if (process.argv.includes("--version")) {
  process.stdout.write("pi 0.0.0-arroba-drill\\n");
  process.exit(0);
}

function argAfter(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : null;
}

function redactedMcpServers(raw) {
  if (!raw) return null;
  try {
    return JSON.stringify(JSON.parse(raw).map((server) => ({
      ...server,
      headers: server?.headers
        ? Object.fromEntries(Object.keys(server.headers).map((key) => [key, "<redacted>"]))
        : undefined,
    })));
  } catch {
    return "<unparseable>";
  }
}

const model = argAfter("--model") ?? "openai-codex/gpt-5.4";
const provider = argAfter("--provider") ?? (model.includes("/") ? model.split("/")[0] : "openai-codex");
const sessionId = argAfter("--resume") ?? process.env.ARROBA_FAKE_PI_SESSION_ID ?? "fake-pi-session";
let pendingAbort = false;

log({
  event: "start",
  argv: process.argv.slice(2),
  cwd: process.cwd(),
  hasMcpServers: Boolean(process.env.ARROBA_PI_MCP_SERVERS),
  mcpServers: redactedMcpServers(process.env.ARROBA_PI_MCP_SERVERS),
});

function send(payload) {
  process.stdout.write(JSON.stringify(payload) + "\\n");
}

function commandResponse(message, success = true) {
  send({
    type: "response",
    id: message.id,
    command: message.type,
    success,
    data: {
      sessionId,
      model: { id: model, provider },
    },
  });
}

function markerFrom(text, fallback) {
  return text.match(/ARROBA_PI_[A-Z0-9_]+|[A-Z0-9]+_(?:BEFORE|AFTER)_RELAY_OK/)?.[0] ?? fallback;
}

function finishPrompt(message) {
  const marker = markerFrom(message.message ?? "", "ARROBA_PI_DEFAULT_MARKER");
  commandResponse(message);
  send({ type: "agent_start", data: { model: { id: model, provider } } });
  if ((message.message ?? "").includes("ARROBA_PI_TOOL_MARKER")) {
    send({
      type: "tool_execution_start",
      toolCallId: "fake-tool-1",
      toolName: "arroba_mcp_fake_ping",
      args: { marker },
    });
    send({
      type: "tool_execution_end",
      toolCallId: "fake-tool-1",
      toolName: "arroba_mcp_fake_ping",
      result: { ok: true, marker },
      isError: false,
    });
  }
  send({
    type: "message_update",
    assistantMessageEvent: { type: "thinking_delta", delta: "fake-pi-thinking" },
  });
  send({
    type: "message_update",
    message: { model: { id: model } },
    assistantMessageEvent: { type: "text_delta", delta: marker },
  });
  send({
    type: "turn_end",
    usage: { input_tokens: 3, output_tokens: 2, total_tokens: 5, context_window: 128000 },
  });
  send({ type: "agent_end", sessionId });
}

const rl = readline.createInterface({ input: process.stdin });
rl.on("line", (line) => {
  if (!line.trim()) return;
  let message;
  try {
    message = JSON.parse(line);
  } catch (error) {
    log({ event: "parse_error", line, error: String(error?.message ?? error) });
    return;
  }
  log({ event: "request", request: message });
  if (message.type === "get_state") {
    commandResponse(message);
  } else if (message.type === "prompt" || message.type === "steer" || message.type === "follow_up") {
    if ((message.message ?? "").includes("ARROBA_PI_ABORT_HANG")) {
      pendingAbort = true;
      commandResponse(message);
      send({ type: "agent_start", data: { model: { id: model, provider } } });
      send({ type: "message_update", assistantMessageEvent: { type: "text_delta", delta: "ARROBA_PI_ABORT_STARTED" } });
      return;
    }
    finishPrompt(message);
  } else if (message.type === "abort") {
    commandResponse(message);
    if (pendingAbort) {
      pendingAbort = false;
      send({ type: "agent_end", sessionId });
    }
  } else {
    commandResponse(message);
  }
});
`
}
