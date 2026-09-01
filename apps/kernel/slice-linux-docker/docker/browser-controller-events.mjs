const DEFAULT_EVENT_CAPACITY = 2_048;
const MAX_EVENT_CAPACITY = 16_384;
const DEFAULT_POLL_LIMIT = 100;
const MAX_POLL_LIMIT = 200;

export class BrowserEventError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "BrowserEventError";
    this.code = code;
  }
}

export class BrowserEventJournal {
  constructor({ capacity = DEFAULT_EVENT_CAPACITY } = {}) {
    if (!Number.isSafeInteger(capacity) || capacity <= 0 || capacity > MAX_EVENT_CAPACITY) {
      throw new BrowserEventError(
        "browser_event_capacity_invalid",
        `browser event capacity must be between 1 and ${MAX_EVENT_CAPACITY}`,
      );
    }
    this.capacity = capacity;
    this.events = [];
    this.nextEventId = 1;
    this.browserGeneration = 0;
  }

  cursor() {
    return this.nextEventId - 1;
  }

  recordCdp(message, context) {
    const browserGeneration = context?.browserGeneration;
    if (!Number.isSafeInteger(browserGeneration) || browserGeneration <= 0) return null;
    if (this.browserGeneration !== browserGeneration) {
      this.browserGeneration = browserGeneration;
      this.events = [];
      this.nextEventId = 1;
    }
    const mapped = mapCdpEvent(message, context);
    if (!mapped) return null;
    const event = {
      event_id: this.nextEventId,
      browser_generation: browserGeneration,
      kind: mapped.kind,
      target_id: mapped.targetId ?? null,
      document_id: mapped.documentId ?? null,
      data: mapped.data,
    };
    this.nextEventId += 1;
    this.events.push(event);
    while (this.events.length > this.capacity) this.events.shift();
    return event;
  }

  poll({ cursor, limit = DEFAULT_POLL_LIMIT, browserGeneration }) {
    if (!Number.isSafeInteger(cursor) || cursor < 0) {
      throw new BrowserEventError(
        "browser_event_cursor_invalid",
        "browser event cursor must be a non-negative integer",
      );
    }
    if (!Number.isSafeInteger(limit) || limit <= 0 || limit > MAX_POLL_LIMIT) {
      throw new BrowserEventError(
        "browser_event_limit_invalid",
        `browser event limit must be between 1 and ${MAX_POLL_LIMIT}`,
      );
    }
    const currentCursor = this.cursor();
    const oldestEventId = this.events[0]?.event_id ?? this.nextEventId;
    if (
      browserGeneration !== this.browserGeneration ||
      cursor > currentCursor ||
      cursor + 1 < oldestEventId
    ) {
      return {
        browser_generation: this.browserGeneration,
        events: [],
        next_cursor: currentCursor,
        replay_gap: true,
      };
    }
    const events = this.events
      .filter((event) => event.event_id > cursor)
      .slice(0, limit);
    return {
      browser_generation: this.browserGeneration,
      events,
      next_cursor: events.at(-1)?.event_id ?? currentCursor,
      replay_gap: false,
    };
  }
}

function mapCdpEvent(message, context) {
  const method = message?.method;
  const params = message?.params ?? {};
  const sessionTargetId = message?.sessionId
    ? context.targetIdForSession?.(message.sessionId)
    : null;
  const currentDocumentId = sessionTargetId
    ? context.documentIdForTarget?.(sessionTargetId)
    : null;
  switch (method) {
    case "Runtime.consoleAPICalled":
      return event("console", sessionTargetId, currentDocumentId, {
        console_type: boundedString(params.type, 64),
        argument_count: boundedCount(params.args?.length),
      });
    case "Network.requestWillBeSent":
      return event("network_request", sessionTargetId, currentDocumentId, {
        request_id: boundedString(params.requestId, 256),
        method: boundedString(params.request?.method, 32),
        url: sanitizedUrl(params.request?.url),
        resource_type: boundedString(params.type, 64),
      });
    case "Network.responseReceived":
      return event("network_response", sessionTargetId, currentDocumentId, {
        request_id: boundedString(params.requestId, 256),
        status: boundedNumber(params.response?.status),
        url: sanitizedUrl(params.response?.url),
        resource_type: boundedString(params.type, 64),
        mime_type: boundedString(params.response?.mimeType, 128),
      });
    case "Network.loadingFailed":
      return event("network_failed", sessionTargetId, currentDocumentId, {
        request_id: boundedString(params.requestId, 256),
        error_text: sanitizedNetworkError(params.errorText),
        canceled: params.canceled === true,
        resource_type: boundedString(params.type, 64),
      });
    case "Page.frameNavigated":
      if (params.frame?.parentId) return null;
      return event("page_navigated", sessionTargetId, params.frame?.loaderId ?? null, {
        url: sanitizedUrl(params.frame?.url),
      });
    case "Page.domContentEventFired":
      return event("dom_content_loaded", sessionTargetId, currentDocumentId, {});
    case "Page.loadEventFired":
      return event("page_loaded", sessionTargetId, currentDocumentId, {});
    case "Page.javascriptDialogOpening":
      return event("dialog_opened", sessionTargetId, currentDocumentId, {
        dialog_type: boundedString(params.type, 32),
        has_message: typeof params.message === "string" && params.message.length > 0,
        has_default_prompt:
          typeof params.defaultPrompt === "string" && params.defaultPrompt.length > 0,
      });
    case "Page.javascriptDialogClosed":
      return event("dialog_closed", sessionTargetId, currentDocumentId, {
        result: params.result === true,
        user_input_present:
          typeof params.userInput === "string" && params.userInput.length > 0,
      });
    case "Target.targetCreated":
    case "Target.targetInfoChanged": {
      const target = params.targetInfo;
      if (target?.type !== "page") return null;
      return event(
        method === "Target.targetCreated" ? "target_created" : "target_changed",
        target.targetId,
        null,
        { url: sanitizedUrl(target.url) },
      );
    }
    case "Target.targetDestroyed":
      return event("target_destroyed", params.targetId ?? null, null, {});
    case "Target.targetCrashed":
      return event("target_crashed", params.targetId ?? null, null, {
        status: boundedString(params.status, 64),
        error_code: Number.isSafeInteger(params.errorCode) ? params.errorCode : null,
      });
    case "Inspector.targetCrashed":
      return event("target_crashed", sessionTargetId, currentDocumentId, {
        status: "crashed",
        error_code: null,
      });
    case "Browser.downloadWillBegin":
      return event("download_started", context.targetIdForFrame?.(params.frameId) ?? null, null, {
        guid: boundedString(params.guid, 128),
        url: sanitizedUrl(params.url),
        suggested_filename: sanitizedFilename(params.suggestedFilename),
      });
    case "Browser.downloadProgress":
      return event("download_progress", context.targetIdForDownload?.(params.guid) ?? null, null, {
        guid: boundedString(params.guid, 128),
        state: boundedString(params.state, 32),
        received_bytes: boundedNumber(params.receivedBytes),
        total_bytes: boundedNumber(params.totalBytes),
      });
    case "Chariox.browserConnected":
      return event("browser_connected", null, null, {});
    case "Chariox.browserDisconnected":
      return event("browser_disconnected", null, null, {});
    default:
      return null;
  }
}

function event(kind, targetId, documentId, data) {
  return { kind, targetId, documentId, data };
}

function sanitizedUrl(value) {
  if (typeof value !== "string" || value.length === 0) return "";
  try {
    const url = new URL(value);
    if (url.protocol === "http:" || url.protocol === "https:") {
      return boundedString(`${url.origin}${url.pathname}`, 2_048);
    }
    if (url.protocol === "about:") return boundedString(value.split(/[?#]/, 1)[0], 128);
    return boundedString(url.protocol, 32);
  } catch {
    return "[invalid]";
  }
}

function sanitizedFilename(value) {
  if (typeof value !== "string") return "";
  const segments = value.split(/[\\/]/);
  return boundedString(segments.at(-1), 255);
}

function sanitizedNetworkError(value) {
  if (typeof value !== "string") return "";
  return /^net::[A-Z0-9_]+$/.test(value) ? value : "[redacted]";
}

function boundedString(value, maxBytes) {
  if (typeof value !== "string") return "";
  let result = "";
  let bytes = 0;
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    const width = codePoint <= 0x7f ? 1 : codePoint <= 0x7ff ? 2 : codePoint <= 0xffff ? 3 : 4;
    if (bytes + width > maxBytes) break;
    result += character;
    bytes += width;
  }
  return result;
}

function boundedCount(value) {
  return Number.isSafeInteger(value) && value >= 0 ? Math.min(value, 10_000) : 0;
}

function boundedNumber(value) {
  return Number.isFinite(value) && value >= 0 ? Math.min(Math.floor(value), Number.MAX_SAFE_INTEGER) : 0;
}
