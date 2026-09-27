#!/usr/bin/env node

import { realpathSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

import { BrowserCdpClient, BrowserControllerError } from "./browser-controller-cdp.mjs";
import { BrowserActionError } from "./browser-controller-actions.mjs";
import {
  BrowserResourceInventoryError,
  observeBrowserResources,
  validateBrowserResourceInventory,
} from "./browser-controller-resources.mjs";

let browserImportModule;

export async function handleBrowserControllerRequest(
  request,
  {
    processId = process.pid,
    browser = new BrowserCdpClient(),
    resourceInventory = observeBrowserResources,
    signal,
  } = {},
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
      const reconciled = await browser.reconcile(request.params?.viewport);
      const observedInventory = reconciled?.resource_inventory
        ?? await resourceInventory();
      return successResponse(
        request.id,
        {
          ...reconciled,
          resource_inventory: validateBrowserResourceInventory(observedInventory),
        },
      );
    }
    if (request.method === "browser.snapshot") {
      return successResponse(
        request.id,
        await browser.snapshot(request.params),
      );
    }
    if (request.method === "browser.tab") {
      return successResponse(
        request.id,
        await browser.manageTab(request.params, { signal }),
      );
    }
    if (request.method === "browser.action") {
      return successResponse(
        request.id,
        await browser.performAction(request.params, { signal }),
      );
    }
    if (request.method === "browser.navigate") {
      return successResponse(
        request.id,
        await browser.navigate(request.params, { signal }),
      );
    }
    if (request.method === "browser.history") {
      return successResponse(
        request.id,
        await browser.manageHistory(request.params, { signal }),
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
        await browser.handleDialog(request.params, { signal }),
      );
    }
    if (request.method === "browser.downloads.configure") {
      return successResponse(
        request.id,
        await browser.configureDownloads(request.params, { signal }),
      );
    }
    if (request.method === "browser.downloads.cancel") {
      return successResponse(request.id, await browser.cancelDownload(request.params));
    }
    if (request.method === "browser.upload") {
      return successResponse(
        request.id,
        await browser.uploadFiles(request.params, { signal }),
      );
    }
    if (request.method === "browser.permission") {
      return successResponse(
        request.id,
        await browser.setPermission(request.params, { signal }),
      );
    }
    if (request.method === "browser.events.poll") {
      return successResponse(
        request.id,
        browser.pollEvents(request.params),
      );
    }
    if (request.method === "browser.cookies.import") {
      const modulePath = process.env.CHARIOX_BROWSER_IMPORT_MODULE
        ?? new URL("../../../browser-session-import/production-destination.mjs",import.meta.url).href;
      browserImportModule ??= import(modulePath);
      const {applyProductionBrowserImport} = await browserImportModule;
      return successResponse(request.id,await applyProductionBrowserImport({
        controller:browser,params:request.params,signal,
      }));
    }
    if (request.method === "browser.cookies.recover") {
      const modulePath = process.env.CHARIOX_BROWSER_IMPORT_MODULE
        ?? new URL("../../../browser-session-import/production-destination.mjs",import.meta.url).href;
      browserImportModule ??= import(modulePath);
      const {recoverProductionBrowserImport} = await browserImportModule;
      return successResponse(request.id,await recoverProductionBrowserImport({controller:browser,params:request.params}));
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
        || error instanceof BrowserActionError
        || error instanceof BrowserResourceInventoryError
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
    resourceInventory = observeBrowserResources,
  } = {}) {
    this.input = input;
    this.output = output;
    this.processId = processId;
    this.browser = browser;
    this.resourceInventory = resourceInventory;
  }

  async run() {
    const server = this;
    const actions = new Map();
    const pendingRequestIds = new Set();
    const waiting = [];
    const targetOperations = new Map();
    const idleWaiters = [];
    let queued = 0;
    let activeOperations = 0;
    let activeBarrier = false;
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
      if (!Number.isSafeInteger(request?.id) || request.id <= 0) {
        this.write(errorResponse(request?.id ?? null, "invalid_request", "request id must be a positive integer"));
        continue;
      }
      if (pendingRequestIds.has(request.id)) {
        this.write(errorResponse(request.id, "controller_busy", "controller queue is full or request id is already pending"));
        continue;
      }
      // Cancellation must be read while the serial browser operation is
      // pending. Its acknowledgement is not the action's terminal response.
      if (request.method === "browser.cancel") {
        pendingRequestIds.add(request.id);
        try {
          const target = request.params?.request_id;
          if (!Number.isSafeInteger(target) || target <= 0) {
            this.write(errorResponse(request.id, "invalid_request", "cancellation requires a positive request_id"));
            continue;
          }
          const action = actions.get(target);
          action?.controller.abort();
          // Ordinary browser operations may be inside a CDP send that cannot
          // observe AbortSignal until the fake/real browser returns. Do not
          // deadlock their cancellation acknowledgement on that send. Cookie
          // import is the exception: its acknowledgement promises that the
          // rollback cleanup has completed before the caller retries.
          if (action?.method === "browser.cookies.import") await action.stopped;
          const accepted = Boolean(action) && (action.method !== "browser.cookies.import"
            || action.response?.result?.status === "rolled_back"
            || action.response?.error?.code === "browser_action_cancelled");
          this.write(successResponse(request.id, { accepted }));
        } finally {
          pendingRequestIds.delete(request.id);
        }
        continue;
      }
      if (queued >= 64) {
        this.write(errorResponse(request.id, "controller_busy", "controller queue is full or request id is already pending"));
        continue;
      }
      let stopAction;
      const controller = ["browser.action", "browser.upload", "browser.downloads.configure", "browser.permission", "browser.tab", "browser.navigate", "browser.history", "browser.dialog", "browser.cookies.import"].includes(request.method) ? new AbortController() : null;
      const action = controller ? {controller,method:request.method,response:null,
        stopped:new Promise(resolve => { stopAction = resolve; })} : null;
      if (action) actions.set(request.id, action);
      pendingRequestIds.add(request.id);
      queued += 1;
      waiting.push({ request, action, stopAction, scheduling: classifyScheduling(request) });
      pump();
      if (request.method === "shutdown") {
        lines.close();
        break;
      }
    }
    await waitUntilIdle();

    function pump() {
      while (!activeBarrier) {
        const barrierIndex = waiting.findIndex(({ scheduling }) => scheduling.kind === "barrier");
        if (barrierIndex === 0) {
          if (activeOperations === 0) {
            startBarrier(waiting.shift());
            continue;
          }
          return;
        }
        let limit = barrierIndex < 0 ? waiting.length : barrierIndex;
        let started = false;
        for (let index = 0; index < limit;) {
          const operation = waiting[index];
          if (!canStart(operation, index)) {
            index += 1;
            continue;
          }
          waiting.splice(index, 1);
          limit -= 1;
          startOrdinary(operation);
          started = true;
        }
        if (!started) return;
      }
    }

    function canStart(operation, index) {
      const { kind, targetId } = operation.scheduling;
      if (!targetId) return true;
      const target = targetOperations.get(targetId) ?? { readers: 0, writer: false };
      const earlierForTarget = waiting.slice(0, index)
        .some((candidate) => candidate.scheduling.targetId === targetId);
      if (kind === "mutation") {
        return !target.writer && target.readers === 0 && !earlierForTarget;
      }
      const earlierMutation = waiting.slice(0, index)
        .some((candidate) => candidate.scheduling.targetId === targetId
          && candidate.scheduling.kind === "mutation");
      return !target.writer && !earlierMutation;
    }

    function startOrdinary(operation) {
      const { kind, targetId } = operation.scheduling;
      if (targetId) {
        const target = targetOperations.get(targetId) ?? { readers: 0, writer: false };
        if (kind === "mutation") target.writer = true;
        else target.readers += 1;
        targetOperations.set(targetId, target);
      }
      activeOperations += 1;
      void execute(operation, false);
    }

    function startBarrier(operation) {
      activeBarrier = true;
      void execute(operation, true);
    }

    async function execute(operation, barrier) {
      const { request, action, stopAction, scheduling } = operation;
      try {
        // Queued cancellation must terminalize before physical browser dispatch.
        const response = action?.controller.signal.aborted
          ? errorResponse(request.id, "browser_action_cancelled", "browser action was cancelled")
          : await handleBrowserControllerRequest(request, {
              processId: server.processId,
              browser: server.browser,
              resourceInventory: server.resourceInventory,
              signal: action?.controller.signal,
            });
        if (action) action.response = response;
        server.write(response);
      } catch (error) {
        server.write(errorResponse(request.id, "browser_controller_internal",
          error instanceof Error ? error.message : String(error)));
      } finally {
        actions.delete(request.id);
        pendingRequestIds.delete(request.id);
        stopAction?.();
        queued -= 1;
        if (barrier) activeBarrier = false;
        else {
          activeOperations -= 1;
          if (scheduling.targetId) {
            const target = targetOperations.get(scheduling.targetId);
            if (scheduling.kind === "mutation") target.writer = false;
            else target.readers -= 1;
            if (target.readers === 0 && !target.writer) targetOperations.delete(scheduling.targetId);
          }
        }
        pump();
        if (queued === 0 && waiting.length === 0 && activeOperations === 0 && !activeBarrier) {
          while (idleWaiters.length > 0) idleWaiters.shift()();
        }
      }
    }

    function waitUntilIdle() {
      if (queued === 0 && waiting.length === 0 && activeOperations === 0 && !activeBarrier) return Promise.resolve();
      return new Promise((resolve) => idleWaiters.push(resolve));
    }
  }

  write(response) {
    this.output.write(`${JSON.stringify(response)}\n`);
  }
}

function classifyScheduling(request) {
  const method = request?.method;
  if (["health", "browser.reconcile", "browser.tab", "browser.downloads.configure",
    "browser.downloads.cancel", "browser.permission", "browser.cookies.import",
    "browser.cookies.recover", "shutdown"].includes(method)) return { kind: "barrier" };
  if (["browser.snapshot", "browser.wait"].includes(method)) return targetScheduling(request, "read");
  if (["browser.action", "browser.navigate", "browser.history", "browser.dialog", "browser.upload"].includes(method)) {
    return targetScheduling(request, "mutation");
  }
  // Unknown or uncertain-scope operations use conservative global barriers.
  return { kind: "barrier" };
}

function targetScheduling(request, kind) {
  const targetId = request.params?.target_id;
  return typeof targetId === "string" && targetId.trim() ? { kind, targetId } : { kind: "barrier" };
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
  const minimumDownloadFreeBytes = parseMinimumDownloadFreeBytes(
    process.env.CHARIOX_SLICE_MIN_FREE_MB,
  );
  const browser = new BrowserCdpClient({
    ...(debuggerEndpoint ? { debuggerEndpoint } : {}),
    downloadDirectory: process.env.CHARIOX_BROWSER_DOWNLOAD_DIR,
    minimumDownloadFreeBytes,
    uploadRoots,
  });
  await new BrowserControllerStdioServer({ browser }).run();
}

export function parseMinimumDownloadFreeBytes(rawValue) {
  const value = rawValue ?? "256";
  if (!/^[0-9]+$/.test(value)) {
    throw new Error("CHARIOX_SLICE_MIN_FREE_MB must be a non-negative integer");
  }
  const megabytes = Number(value);
  const bytes = megabytes * 1024 * 1024;
  if (!Number.isSafeInteger(megabytes) || !Number.isSafeInteger(bytes)) {
    throw new Error("CHARIOX_SLICE_MIN_FREE_MB is outside the supported range");
  }
  return bytes;
}

const invokedPath = process.argv[1] ? realpathSync(process.argv[1]) : null;
if (invokedPath === realpathSync(fileURLToPath(import.meta.url))) {
  runCli().catch((error) => {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  });
}
