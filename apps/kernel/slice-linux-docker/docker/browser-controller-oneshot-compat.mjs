const MAX_INPUT_BYTES = 64 * 1024;
const DEFAULT_WAIT_TIMEOUT_MS = 10_000;

export const ONESHOT_COMPAT_ERROR_CODES = Object.freeze({
  INVALID_COMMAND: "ONESHOT_COMPAT_INVALID_COMMAND",
  INVALID_ARGUMENT: "ONESHOT_COMPAT_INVALID_ARGUMENT",
  INPUT_TOO_LARGE: "ONESHOT_COMPAT_INPUT_TOO_LARGE",
  OUTPUT_INVALID: "ONESHOT_COMPAT_OUTPUT_INVALID",
});

export class BrowserOneShotCompatibilityError extends Error {
  constructor(code) {
    super(code);
    this.name = "BrowserOneShotCompatibilityError";
    this.code = code;
  }
}

function fail(code) {
  throw new BrowserOneShotCompatibilityError(code);
}

function assertExactCount(args, minimum, maximum = minimum) {
  if (args.length < minimum || args.length > maximum) {
    fail(ONESHOT_COMPAT_ERROR_CODES.INVALID_ARGUMENT);
  }
}

function targetArguments(target) {
  if (typeof target !== "string" || target.length === 0) {
    fail(ONESHOT_COMPAT_ERROR_CODES.INVALID_ARGUMENT);
  }
  return /^(?:field|button|link|element):[A-Za-z0-9._:-]+$/.test(target)
    ? { field_id: target }
    : { selector: target };
}

function timeout(value) {
  if (value === undefined) return DEFAULT_WAIT_TIMEOUT_MS;
  if (!/^[0-9]+$/.test(value)) {
    fail(ONESHOT_COMPAT_ERROR_CODES.INVALID_ARGUMENT);
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed)) {
    fail(ONESHOT_COMPAT_ERROR_CODES.INVALID_ARGUMENT);
  }
  return parsed;
}

function assertInputBounded(argv, stdin) {
  let encoded;
  try {
    encoded = JSON.stringify({ argv, stdin });
  } catch {
    fail(ONESHOT_COMPAT_ERROR_CODES.INVALID_ARGUMENT);
  }
  if (Buffer.byteLength(encoded, "utf8") > MAX_INPUT_BYTES) {
    fail(ONESHOT_COMPAT_ERROR_CODES.INPUT_TOO_LARGE);
  }
}

export function parseOneShotBrowserCommand(rawArgv, options = {}) {
  if (
    !Array.isArray(rawArgv) ||
    rawArgv.some((value) => typeof value !== "string") ||
    options === null ||
    typeof options !== "object" ||
    Array.isArray(options)
  ) {
    fail(ONESHOT_COMPAT_ERROR_CODES.INVALID_ARGUMENT);
  }
  const stdin = options.stdin ?? "";
  if (typeof stdin !== "string") {
    fail(ONESHOT_COMPAT_ERROR_CODES.INVALID_ARGUMENT);
  }
  assertInputBounded(rawArgv, stdin);

  const [command, ...args] = rawArgv;
  if (command === "status") {
    assertExactCount(args, 0);
    return { tool_name: "chariox.slice_browser_status", arguments: {}, output: "json" };
  }
  if (command === "find") {
    assertExactCount(args, 1, 2);
    return {
      tool_name: "chariox.slice_browser_find",
      arguments: { query: args[0], kind: args[1] ?? "any" },
      output: "json",
    };
  }
  if (command === "fill" || command === "fill-stdin") {
    if (command === "fill") {
      assertExactCount(args, 2, Number.MAX_SAFE_INTEGER);
    } else {
      assertExactCount(args, 1);
    }
    return {
      tool_name: "chariox.slice_browser_fill",
      arguments: {
        ...targetArguments(args[0]),
        text: command === "fill" ? args.slice(1).join(" ") : stdin,
      },
      output: "json",
    };
  }
  if (command === "click-selector") {
    assertExactCount(args, 1);
    return {
      tool_name: "chariox.slice_browser_click",
      arguments: targetArguments(args[0]),
      output: "json",
    };
  }
  if (command === "submit") {
    assertExactCount(args, 0, 1);
    return {
      tool_name: "chariox.slice_browser_submit",
      arguments: args.length === 0 ? {} : targetArguments(args[0]),
      output: "json",
    };
  }
  if (command === "dialog") {
    assertExactCount(args, 1, 2);
    return {
      tool_name: "chariox.slice_browser_dialog",
      arguments: {
        action: args[0],
        ...(args[1] === undefined || args[1] === "" ? {} : { prompt_text: args[1] }),
      },
      output: "json",
    };
  }
  if (command === "text") {
    assertExactCount(args, 0);
    return { tool_name: "chariox.slice_browser_text", arguments: {}, output: "text" };
  }
  if (command === "wait-text") {
    assertExactCount(args, 1, 2);
    return {
      tool_name: "chariox.slice_browser_wait_for_text",
      arguments: { text: args[0], timeout_ms: timeout(args[1]) },
      output: "json",
    };
  }
  if (command === "wait-selector") {
    assertExactCount(args, 1, 2);
    return {
      tool_name: "chariox.slice_browser_wait_for_selector",
      arguments: { selector: args[0], timeout_ms: timeout(args[1]) },
      output: "json",
    };
  }
  if (command === "wait-idle") {
    assertExactCount(args, 0, 1);
    return {
      tool_name: "chariox.slice_browser_wait_for_idle",
      arguments: { timeout_ms: timeout(args[0]) },
      output: "json",
    };
  }
  fail(ONESHOT_COMPAT_ERROR_CODES.INVALID_COMMAND);
}

export class BrowserOneShotCompatibilityAdapter {
  constructor(runtimeAdapter) {
    if (!runtimeAdapter || typeof runtimeAdapter.invoke !== "function") {
      throw new TypeError("runtimeAdapter.invoke must be a function");
    }
    this.runtimeAdapter = runtimeAdapter;
  }

  async invoke(argv, options = {}) {
    const request = parseOneShotBrowserCommand(argv, options);
    const response = await this.runtimeAdapter.invoke(request.tool_name, request.arguments);
    const result = response?.result;
    if (request.output === "text") {
      if (typeof result?.text !== "string") {
        fail(ONESHOT_COMPAT_ERROR_CODES.OUTPUT_INVALID);
      }
      return { exit_code: 0, stdout: result.text };
    }
    let stdout;
    try {
      stdout = JSON.stringify(result);
    } catch {
      fail(ONESHOT_COMPAT_ERROR_CODES.OUTPUT_INVALID);
    }
    if (typeof stdout !== "string") {
      fail(ONESHOT_COMPAT_ERROR_CODES.OUTPUT_INVALID);
    }
    return {
      exit_code: result?.ok === false ? 1 : 0,
      stdout,
    };
  }
}
