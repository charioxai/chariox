#!/usr/bin/env node

import { realpathSync } from "node:fs";
import process from "node:process";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

export function handleBrowserControllerRequest(request, processId = process.pid) {
  if (!request || !Number.isSafeInteger(request.id) || request.id <= 0) {
    return errorResponse(request?.id ?? null, "invalid_request", "request id must be a positive integer");
  }
  if (request.method === "health") {
    return {
      id: request.id,
      ok: true,
      result: {
        state: "ready",
        process_id: processId,
        diagnostic_code: null,
      },
    };
  }
  if (request.method === "shutdown") {
    return {
      id: request.id,
      ok: true,
      result: {
        state: "stopped",
        process_id: null,
        diagnostic_code: null,
      },
    };
  }
  return errorResponse(
    request.id,
    "unknown_method",
    `unknown browser controller method ${JSON.stringify(request.method)}`,
  );
}

export class BrowserControllerStdioServer {
  constructor({ input = process.stdin, output = process.stdout, processId = process.pid } = {}) {
    this.input = input;
    this.output = output;
    this.processId = processId;
  }

  async run() {
    const lines = readline.createInterface({
      input: this.input,
      crlfDelay: Infinity,
      terminal: false,
    });
    for await (const line of lines) {
      if (!line.trim()) {
        continue;
      }
      let request;
      try {
        request = JSON.parse(line);
      } catch (error) {
        this.write(
          errorResponse(
            null,
            "invalid_json",
            error instanceof Error ? error.message : String(error),
          ),
        );
        continue;
      }
      const response = handleBrowserControllerRequest(request, this.processId);
      this.write(response);
      if (request.method === "shutdown" && response.ok) {
        lines.close();
        return;
      }
    }
  }

  write(response) {
    this.output.write(`${JSON.stringify(response)}\n`);
  }
}

function errorResponse(id, code, message) {
  return {
    id,
    ok: false,
    error: { code, message },
  };
}

async function runCli() {
  const command = process.argv[2] ?? "stdio";
  if (command !== "stdio") {
    throw new Error("usage: browser-controller.mjs stdio");
  }
  await new BrowserControllerStdioServer().run();
}

const invokedPath = process.argv[1] ? realpathSync(process.argv[1]) : null;
if (invokedPath === realpathSync(fileURLToPath(import.meta.url))) {
  runCli().catch((error) => {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  });
}
