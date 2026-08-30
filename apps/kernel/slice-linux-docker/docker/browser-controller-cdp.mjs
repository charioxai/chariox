const DEFAULT_DEBUGGER_ENDPOINT = "http://127.0.0.1:9222";
const DEFAULT_REQUEST_TIMEOUT_MS = 5_000;

export class BrowserControllerError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "BrowserControllerError";
    this.code = code;
  }
}

export class BrowserCdpClient {
  constructor({
    debuggerEndpoint = DEFAULT_DEBUGGER_ENDPOINT,
    requestTimeoutMs = DEFAULT_REQUEST_TIMEOUT_MS,
    fetchImpl = globalThis.fetch,
    webSocketFactory = (url) => new WebSocket(url),
    connectionFactory,
  } = {}) {
    this.debuggerEndpoint = new URL(debuggerEndpoint);
    this.requestTimeoutMs = requestTimeoutMs;
    this.fetchImpl = fetchImpl;
    this.webSocketFactory = webSocketFactory;
    this.connectionFactory = connectionFactory;
    this.connection = null;
    this.browserGeneration = 0;
    this.sessionsByTarget = new Map();
  }

  async reconcile(rawViewport) {
    const viewport = canonicalViewport(rawViewport);
    const connection = await this.ensureConnection();
    try {
      const { targetInfos = [] } = await connection.send("Target.getTargets");
      const pages = targetInfos.filter(
        (target) => target?.type === "page" && typeof target.targetId === "string",
      );
      const pageTargetIds = new Set(pages.map((target) => target.targetId));
      for (const targetId of this.sessionsByTarget.keys()) {
        if (!pageTargetIds.has(targetId)) {
          this.sessionsByTarget.delete(targetId);
        }
      }
      const inspected = await Promise.all(
        pages.map((target) => this.inspectPage(connection, target, viewport)),
      );
      const focused = inspected.find((tab) => tab.focused)?.target_id ?? null;
      return {
        browser_generation: this.browserGeneration,
        tabs: inspected.map(({ focused: _focused, ...tab }) => tab),
        focused_target_id: focused,
        viewport,
      };
    } catch (error) {
      if (!connection.isOpen()) {
        this.connection = null;
        this.sessionsByTarget.clear();
      }
      throw normalizeControllerError(error);
    }
  }

  async close() {
    const connection = this.connection;
    this.connection = null;
    this.sessionsByTarget.clear();
    if (connection) {
      await connection.close();
    }
  }

  async ensureConnection() {
    if (this.connection?.isOpen()) {
      return this.connection;
    }
    this.sessionsByTarget.clear();
    this.connection = this.connectionFactory
      ? await this.connectionFactory()
      : await connectToBrowser({
          debuggerEndpoint: this.debuggerEndpoint,
          requestTimeoutMs: this.requestTimeoutMs,
          fetchImpl: this.fetchImpl,
          webSocketFactory: this.webSocketFactory,
        });
    this.browserGeneration += 1;
    return this.connection;
  }

  async inspectPage(connection, target, viewport) {
    let sessionId = this.sessionsByTarget.get(target.targetId);
    if (!sessionId) {
      const attached = await connection.send("Target.attachToTarget", {
        targetId: target.targetId,
        flatten: true,
      });
      if (typeof attached?.sessionId !== "string" || !attached.sessionId) {
        throw new BrowserControllerError(
          "browser_attach_failed",
          `browser target ${JSON.stringify(target.targetId)} did not return a session`,
        );
      }
      sessionId = attached.sessionId;
      this.sessionsByTarget.set(target.targetId, sessionId);
    }
    await connection.send(
      "Emulation.setDeviceMetricsOverride",
      deviceMetricsFor(viewport),
      sessionId,
    );
    const [frameTree, focus] = await Promise.all([
      connection.send("Page.getFrameTree", {}, sessionId),
      connection.send(
        "Runtime.evaluate",
        {
          expression: "document.visibilityState === 'visible'",
          returnByValue: true,
          awaitPromise: false,
        },
        sessionId,
      ),
    ]);
    const documentId = frameTree?.frameTree?.frame?.loaderId;
    if (typeof documentId !== "string" || !documentId) {
      throw new BrowserControllerError(
        "browser_document_identity_missing",
        `browser target ${JSON.stringify(target.targetId)} has no top-level loader identity`,
      );
    }
    return {
      target_id: target.targetId,
      document_id: documentId,
      url: typeof target.url === "string" ? target.url : "",
      title: typeof target.title === "string" ? target.title : "",
      focused: focus?.result?.value === true,
    };
  }
}

export class CdpConnection {
  constructor(socket, requestTimeoutMs = DEFAULT_REQUEST_TIMEOUT_MS) {
    this.socket = socket;
    this.requestTimeoutMs = requestTimeoutMs;
    this.nextRequestId = 1;
    this.pending = new Map();
    this.closed = false;
    socket.addEventListener("message", (event) => this.receive(event.data));
    socket.addEventListener("close", () => this.failPending("browser_cdp_disconnected"));
    socket.addEventListener("error", () => this.failPending("browser_cdp_socket_error"));
  }

  isOpen() {
    return !this.closed && this.socket.readyState === 1;
  }

  send(method, params = {}, sessionId) {
    if (!this.isOpen()) {
      return Promise.reject(
        new BrowserControllerError("browser_cdp_disconnected", "browser CDP connection is closed"),
      );
    }
    const id = this.nextRequestId++;
    const payload = { id, method, params };
    if (sessionId) {
      payload.sessionId = sessionId;
    }
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id);
        reject(
          new BrowserControllerError(
            "browser_cdp_timeout",
            `browser CDP method ${method} timed out after ${this.requestTimeoutMs}ms`,
          ),
        );
      }, this.requestTimeoutMs);
      this.pending.set(id, { method, resolve, reject, timeout });
      try {
        this.socket.send(JSON.stringify(payload));
      } catch (error) {
        clearTimeout(timeout);
        this.pending.delete(id);
        reject(normalizeControllerError(error));
      }
    });
  }

  async close() {
    if (this.closed) {
      return;
    }
    this.closed = true;
    this.failPending("browser_cdp_shutdown");
    if (this.socket.readyState === 0 || this.socket.readyState === 1) {
      this.socket.close();
    }
  }

  receive(rawMessage) {
    let message;
    try {
      message = JSON.parse(String(rawMessage));
    } catch {
      this.failPending("browser_cdp_invalid_json");
      return;
    }
    if (!Number.isSafeInteger(message?.id)) {
      return;
    }
    const pending = this.pending.get(message.id);
    if (!pending) {
      return;
    }
    this.pending.delete(message.id);
    clearTimeout(pending.timeout);
    if (message.error) {
      pending.reject(
        new BrowserControllerError(
          "browser_cdp_command_failed",
          `${pending.method}: ${String(message.error.message ?? "CDP command failed")}`,
        ),
      );
      return;
    }
    pending.resolve(message.result ?? {});
  }

  failPending(code) {
    this.closed = true;
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(new BrowserControllerError(code, "browser CDP connection closed"));
    }
    this.pending.clear();
  }
}

export function assertPrivateDebuggerUrl(rawUrl, debuggerEndpoint = DEFAULT_DEBUGGER_ENDPOINT) {
  const url = new URL(rawUrl);
  const endpoint = new URL(debuggerEndpoint);
  if (
    url.protocol !== "ws:" ||
    !isLoopbackHost(url.hostname) ||
    normalizedPort(url) !== normalizedPort(endpoint)
  ) {
    throw new BrowserControllerError(
      "browser_debugger_endpoint_unsafe",
      "browser debugger WebSocket must remain on the configured loopback port",
    );
  }
  return url.toString();
}

function canonicalViewport(viewport) {
  const keys = [
    "css_width",
    "css_height",
    "device_scale_factor",
    "desktop_pixel_width",
    "desktop_pixel_height",
  ];
  const canonical = {};
  for (const key of keys) {
    const value = viewport?.[key];
    if (!Number.isSafeInteger(value) || value <= 0) {
      throw new BrowserControllerError(
        "browser_viewport_invalid",
        `browser viewport ${key} must be a positive integer`,
      );
    }
    canonical[key] = value;
  }
  return canonical;
}

function deviceMetricsFor(viewport) {
  return {
    width: viewport.css_width,
    height: viewport.css_height,
    deviceScaleFactor: viewport.device_scale_factor,
    mobile: false,
    screenWidth: Math.floor(viewport.desktop_pixel_width / viewport.device_scale_factor),
    screenHeight: Math.floor(viewport.desktop_pixel_height / viewport.device_scale_factor),
  };
}

async function connectToBrowser({
  debuggerEndpoint,
  requestTimeoutMs,
  fetchImpl,
  webSocketFactory,
}) {
  let response;
  try {
    response = await fetchImpl(new URL("/json/version", debuggerEndpoint), {
      signal: AbortSignal.timeout(requestTimeoutMs),
    });
  } catch (error) {
    throw new BrowserControllerError(
      "browser_debugger_unavailable",
      `browser debugger discovery failed: ${String(error?.message ?? error)}`,
    );
  }
  if (!response.ok) {
    throw new BrowserControllerError(
      "browser_debugger_unavailable",
      `browser debugger discovery returned HTTP ${response.status}`,
    );
  }
  const discovery = await response.json();
  if (typeof discovery?.webSocketDebuggerUrl !== "string") {
    throw new BrowserControllerError(
      "browser_debugger_invalid",
      "browser debugger discovery omitted its WebSocket URL",
    );
  }
  const debuggerUrl = assertPrivateDebuggerUrl(
    discovery.webSocketDebuggerUrl,
    debuggerEndpoint,
  );
  const socket = webSocketFactory(debuggerUrl);
  await waitForSocketOpen(socket, requestTimeoutMs);
  return new CdpConnection(socket, requestTimeoutMs);
}

function waitForSocketOpen(socket, timeoutMs) {
  if (socket.readyState === 1) {
    return Promise.resolve();
  }
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(
        new BrowserControllerError(
          "browser_cdp_timeout",
          `browser CDP socket did not open within ${timeoutMs}ms`,
        ),
      );
    }, timeoutMs);
    const opened = () => {
      cleanup();
      resolve();
    };
    const failed = () => {
      cleanup();
      reject(
        new BrowserControllerError("browser_cdp_socket_error", "browser CDP socket failed"),
      );
    };
    const cleanup = () => {
      clearTimeout(timeout);
      socket.removeEventListener("open", opened);
      socket.removeEventListener("error", failed);
    };
    socket.addEventListener("open", opened, { once: true });
    socket.addEventListener("error", failed, { once: true });
  });
}

function isLoopbackHost(hostname) {
  const normalized = hostname.replace(/^\[|\]$/g, "").toLowerCase();
  return normalized === "127.0.0.1" || normalized === "localhost" || normalized === "::1";
}

function normalizedPort(url) {
  if (url.port) {
    return url.port;
  }
  return url.protocol === "https:" || url.protocol === "wss:" ? "443" : "80";
}

function normalizeControllerError(error) {
  if (error instanceof BrowserControllerError) {
    return error;
  }
  return new BrowserControllerError(
    "browser_controller_internal",
    String(error?.message ?? error),
  );
}
