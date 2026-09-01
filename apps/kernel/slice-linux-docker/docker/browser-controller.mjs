#!/usr/bin/env node

import { realpathSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

import { BrowserCdpClient, BrowserControllerError } from "./browser-controller-cdp.mjs";

export async function handleBrowserControllerRequest(
  request,
  { processId = process.pid, browser = new BrowserCdpClient() } = {},
) {
  if (!request || !Number.isSafeInteger(request.id) || request.id <= 0) {
    return errorResponse(request?.id ?? null, "invalid_request", "request id must be a positive integer");
  }
  try {
    if (request.method === "health") {
      return successResponse(request.id, {
        state: "ready",
        process_id: processId,
        diagnostic_code: null,
      });
    }
    if (request.method === "browser.reconcile") {
      return successResponse(
        request.id,
        await browser.reconcile(request.params?.viewport),
      );
    }
    if (request.method === "browser.snapshot") {
      return successResponse(
        request.id,
        await browser.snapshot(request.params),
      );
    }
    if (request.method === "browser.action") {
      return successResponse(
        request.id,
        await browser.performAction(request.params),
      );
    }
    if (request.method === "browser.navigate") {
      return successResponse(
        request.id,
        await browser.navigate(request.params),
      );
    }
    if (request.method === "browser.wait") {
      return successResponse(
        request.id,
        await browser.wait(request.params),
      );
    }
    if (request.method === "browser.dialog") {
      return successResponse(
        request.id,
        await browser.handleDialog(request.params),
      );
    }
    if (request.method === "browser.downloads.configure") {
      return successResponse(
        request.id,
        await browser.configureDownloads(request.params),
      );
    }
    if (request.method === "browser.upload") {
      return successResponse(
        request.id,
        await browser.uploadFiles(request.params),
      );
    }
    if (request.method === "browser.permission") {
      return successResponse(
        request.id,
        await browser.setPermission(request.params),
      );
    }
    if (request.method === "browser.events.poll") {
      return successResponse(
        request.id,
        browser.pollEvents(request.params),
      );
    }
    if (request.method === "shutdown") {
      await browser.close();
      return successResponse(request.id, {
        state: "stopped",
        process_id: null,
        diagnostic_code: null,
      });
    }
    return errorResponse(
      request.id,
      "unknown_method",
      `unknown browser controller method ${JSON.stringify(request.method)}`,
    );
  } catch (error) {
    const code =
      error instanceof BrowserControllerError
        ? error.code
        : "browser_controller_internal";
    return errorResponse(
      request.id,
      code,
      error instanceof Error ? error.message : String(error),
    );
  }
}

export class BrowserControllerStdioServer {
  constructor({
    input = process.stdin,
    output = process.stdout,
    processId = process.pid,
    browser = new BrowserCdpClient(),
  } = {}) {
    this.input = input;
    this.output = output;
    this.processId = processId;
    this.browser = browser;
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
      const response = await handleBrowserControllerRequest(request, {
        processId: this.processId,
        browser: this.browser,
      });
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

function successResponse(id, result) {
  return { id, ok: true, result };
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
  const debuggerEndpoint = process.env.CHARIOX_BROWSER_DEBUGGER_ENDPOINT;
  const uploadRoots = (process.env.CHARIOX_BROWSER_UPLOAD_ROOTS ?? "")
    .split(path.delimiter)
    .filter(Boolean);
  const browser = new BrowserCdpClient({
    ...(debuggerEndpoint ? { debuggerEndpoint } : {}),
    downloadDirectory: process.env.CHARIOX_BROWSER_DOWNLOAD_DIR,
    uploadRoots,
  });
  await new BrowserControllerStdioServer({ browser }).run();
}

const invokedPath = process.argv[1] ? realpathSync(process.argv[1]) : null;
if (invokedPath === realpathSync(fileURLToPath(import.meta.url))) {
  runCli().catch((error) => {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  });
}
