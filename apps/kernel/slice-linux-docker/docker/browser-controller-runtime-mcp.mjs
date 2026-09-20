const MAX_REQUEST_BYTES = 64 * 1024;
const MAX_RESULT_BYTES = 1024 * 1024;
const MAX_STRING_BYTES = 64 * 1024;
const MIN_TIMEOUT_MS = 100;
const MAX_TIMEOUT_MS = 60_000;

export const RUNTIME_MCP_ERROR_CODES = Object.freeze({
  INVALID_TOOL: "RUNTIME_MCP_INVALID_TOOL",
  INVALID_ARGUMENT: "RUNTIME_MCP_INVALID_ARGUMENT",
  REQUEST_TOO_LARGE: "RUNTIME_MCP_REQUEST_TOO_LARGE",
  RESULT_INVALID: "RUNTIME_MCP_RESULT_INVALID",
  RESULT_TOO_LARGE: "RUNTIME_MCP_RESULT_TOO_LARGE",
});

export class BrowserRuntimeMcpError extends Error {
  constructor(code) {
    super(code);
    this.name = "BrowserRuntimeMcpError";
    this.code = code;
  }
}

const TOOL_SUFFIXES = Object.freeze([
  "status",
  "find",
  "fill",
  "click",
  "submit",
  "dialog",
  "text",
  "wait_for_text",
  "wait_for_selector",
  "wait_for_idle",
]);

export const RUNTIME_MCP_TOOL_NAMES = Object.freeze(
  Object.fromEntries(TOOL_SUFFIXES.map((suffix) => [suffix, `chariox.slice_browser_${suffix}`])),
);

const PORT_METHODS = Object.freeze({
  status: "status",
  find: "find",
  fill: "fill",
  click: "click",
  submit: "submit",
  dialog: "dialog",
  text: "text",
  wait_for_text: "waitForText",
  wait_for_selector: "waitForSelector",
  wait_for_idle: "waitForIdle",
});

const FORBIDDEN_RESULT_KEYS = new Set([
  "backend_node_id",
  "backendNodeId",
  "browser_context_id",
  "browserContextId",
  "object_id",
  "objectId",
  "session_id",
  "sessionId",
  "target_id",
  "targetId",
  "websocket_url",
  "websocketUrl",
  "webSocketDebuggerUrl",
]);

function fail(code) {
  throw new BrowserRuntimeMcpError(code);
}

function isPlainObject(value) {
  if (value === null || typeof value !== "object") return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function assertExactKeys(value, allowed, required = []) {
  if (!isPlainObject(value)) fail(RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT);
  const allowedKeys = new Set(allowed);
  if (Object.keys(value).some((key) => !allowedKeys.has(key))) {
    fail(RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT);
  }
  if (required.some((key) => !Object.prototype.hasOwnProperty.call(value, key))) {
    fail(RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT);
  }
}

function boundedString(value, { required = false, maximum = MAX_STRING_BYTES } = {}) {
  if (value === undefined && !required) return undefined;
  if (
    typeof value !== "string" ||
    (required && value.length === 0) ||
    Buffer.byteLength(value, "utf8") > maximum ||
    /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(value)
  ) {
    fail(RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT);
  }
  return value;
}

function optionalTimeout(value) {
  if (value === undefined) return undefined;
  if (!Number.isSafeInteger(value) || value < MIN_TIMEOUT_MS || value > MAX_TIMEOUT_MS) {
    fail(RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT);
  }
  return value;
}

function optionalTarget(value, key) {
  if (value === undefined) return undefined;
  const target = boundedString(value, { required: true, maximum: 8 * 1024 });
  if (target.trim().length === 0) fail(RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT);
  return target;
}

function targetArgs(value, { required = false } = {}) {
  const selector = optionalTarget(value.selector, "selector");
  const fieldId = optionalTarget(value.field_id, "field_id");
  if ((required && selector === undefined && fieldId === undefined) ||
      (selector !== undefined && fieldId !== undefined)) {
    fail(RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT);
  }
  return {
    ...(selector === undefined ? {} : { selector }),
    ...(fieldId === undefined ? {} : { field_id: fieldId }),
  };
}

function validateArguments(suffix, raw) {
  const value = raw ?? {};
  if (suffix === "status" || suffix === "text") {
    assertExactKeys(value, []);
    return {};
  }
  if (suffix === "find") {
    assertExactKeys(value, ["query", "kind"], ["query"]);
    const query = boundedString(value.query, { required: true, maximum: 8 * 1024 });
    const kind = value.kind ?? "any";
    if (!["field", "button", "link", "any"].includes(kind)) {
      fail(RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT);
    }
    return { query, kind };
  }
  if (suffix === "fill") {
    assertExactKeys(value, ["selector", "field_id", "text"], ["text"]);
    if (typeof value.text !== "string") {
      fail(RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT);
    }
    return {
      ...targetArgs(value, { required: true }),
      text: boundedString(value.text, { required: false }),
    };
  }
  if (suffix === "click") {
    assertExactKeys(value, ["selector", "field_id"]);
    return targetArgs(value, { required: true });
  }
  if (suffix === "submit") {
    assertExactKeys(value, ["selector", "field_id"]);
    return targetArgs(value);
  }
  if (suffix === "dialog") {
    assertExactKeys(value, ["action", "prompt_text"], ["action"]);
    if (!["accept", "dismiss"].includes(value.action)) {
      fail(RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT);
    }
    const promptText = boundedString(value.prompt_text, { maximum: 8 * 1024 });
    if (value.action === "dismiss" && promptText !== undefined) {
      fail(RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT);
    }
    return {
      action: value.action,
      ...(promptText === undefined ? {} : { prompt_text: promptText }),
    };
  }
  if (suffix === "wait_for_text") {
    assertExactKeys(value, ["text", "timeout_ms"], ["text"]);
    const timeoutMs = optionalTimeout(value.timeout_ms);
    return {
      text: boundedString(value.text, { required: true }),
      ...(timeoutMs === undefined ? {} : { timeout_ms: timeoutMs }),
    };
  }
  if (suffix === "wait_for_selector") {
    assertExactKeys(value, ["selector", "timeout_ms"], ["selector"]);
    const timeoutMs = optionalTimeout(value.timeout_ms);
    return {
      selector: boundedString(value.selector, { required: true, maximum: 8 * 1024 }),
      ...(timeoutMs === undefined ? {} : { timeout_ms: timeoutMs }),
    };
  }
  if (suffix === "wait_for_idle") {
    assertExactKeys(value, ["timeout_ms"]);
    const timeoutMs = optionalTimeout(value.timeout_ms);
    return timeoutMs === undefined ? {} : { timeout_ms: timeoutMs };
  }
  fail(RUNTIME_MCP_ERROR_CODES.INVALID_TOOL);
}

function suffixForToolName(name) {
  if (typeof name !== "string") fail(RUNTIME_MCP_ERROR_CODES.INVALID_TOOL);
  for (const suffix of TOOL_SUFFIXES) {
    const canonical = `chariox.slice_browser_${suffix}`;
    if (
      name === canonical ||
      name === `slice_browser_${suffix}` ||
      name === `chariox_slice_browser_${suffix}` ||
      name === `mcp__chariox__slice_browser_${suffix}` ||
      name === `mcp__chariox__chariox_slice_browser_${suffix}`
    ) {
      return suffix;
    }
  }
  fail(RUNTIME_MCP_ERROR_CODES.INVALID_TOOL);
}

function assertRequestBounded(toolName, args) {
  let encoded;
  try {
    encoded = JSON.stringify({ tool_name: toolName, arguments: args });
  } catch {
    fail(RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT);
  }
  if (Buffer.byteLength(encoded, "utf8") > MAX_REQUEST_BYTES) {
    fail(RUNTIME_MCP_ERROR_CODES.REQUEST_TOO_LARGE);
  }
}

function assertPublicResult(value, seen = new Set()) {
  if (value === null || typeof value === "boolean" || typeof value === "string") return;
  if (typeof value === "number") {
    if (!Number.isFinite(value)) fail(RUNTIME_MCP_ERROR_CODES.RESULT_INVALID);
    return;
  }
  if (typeof value !== "object" || seen.has(value)) {
    fail(RUNTIME_MCP_ERROR_CODES.RESULT_INVALID);
  }
  seen.add(value);
  if (Array.isArray(value)) {
    for (const item of value) assertPublicResult(item, seen);
  } else if (isPlainObject(value)) {
    for (const [key, item] of Object.entries(value)) {
      if (FORBIDDEN_RESULT_KEYS.has(key)) fail(RUNTIME_MCP_ERROR_CODES.RESULT_INVALID);
      assertPublicResult(item, seen);
    }
  } else {
    fail(RUNTIME_MCP_ERROR_CODES.RESULT_INVALID);
  }
  seen.delete(value);
}

function validateResult(value) {
  assertPublicResult(value);
  let encoded;
  try {
    encoded = JSON.stringify(value);
  } catch {
    fail(RUNTIME_MCP_ERROR_CODES.RESULT_INVALID);
  }
  if (Buffer.byteLength(encoded, "utf8") > MAX_RESULT_BYTES) {
    fail(RUNTIME_MCP_ERROR_CODES.RESULT_TOO_LARGE);
  }
  return value;
}

export function canonicalRuntimeMcpToolName(name) {
  return RUNTIME_MCP_TOOL_NAMES[suffixForToolName(name)];
}

export class BrowserRuntimeMcpAdapter {
  constructor(port) {
    if (!isPlainObject(port)) throw new TypeError("port must be a plain object");
    for (const method of Object.values(PORT_METHODS)) {
      if (typeof port[method] !== "function") {
        throw new TypeError(`port.${method} must be a function`);
      }
    }
    this.port = port;
  }

  supports(toolName) {
    try {
      suffixForToolName(toolName);
      return true;
    } catch {
      return false;
    }
  }

  async invoke(toolName, rawArguments = {}) {
    const suffix = suffixForToolName(toolName);
    const canonicalName = RUNTIME_MCP_TOOL_NAMES[suffix];
    const args = validateArguments(suffix, rawArguments);
    assertRequestBounded(canonicalName, args);
    const result = await this.port[PORT_METHODS[suffix]](args);
    return {
      tool_name: canonicalName,
      result: validateResult(result),
    };
  }
}
