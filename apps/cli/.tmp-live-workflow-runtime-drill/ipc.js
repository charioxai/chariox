import net from "node:net";
import { createCipheriv, createDecipheriv, createECDH, hkdfSync, randomBytes, randomUUID } from "node:crypto";
import WebSocket from "ws";
const IPC_TIMEOUT_MS = 120_000;
const DEFAULT_KERNEL_EVENT_STALE_MS = 0;
const DEFAULT_KERNEL_PING_INTERVAL_MS = 5_000;
const DEFAULT_KERNEL_MAX_MISSED_PONGS = 2;
export class LocalIpcError extends Error {
  constructor(operation, message, code = null, retryable = false) {
    super(`kernel transport \`${operation}\` failed: ${message}`);
    this.operation = operation;
    this.code = code;
    this.retryable = retryable;
    this.name = "LocalIpcError";
  }
}
export class LocalIpcClient {
  controlWebsocket = null;
  eventWebsocket = null;
  controlWebsocketConnectPromise = null;
  eventWebsocketConnectPromise = null;
  pending = new Map();
  eventHandlers = new Set();
  activeKernelSubscription = null;
  reconnectTimeout = null;
  reconnectDelayMs = 250;
  lastReceivedEventId = null;
  lastKernelEventAtMs = 0;
  kernelEventWatchdog = null;
  controlHeartbeat = null;
  eventHeartbeat = null;
  missedControlPongs = 0;
  missedEventPongs = 0;
  suppressNextControlCloseEvent = false;
  suppressNextEventCloseEvent = false;
  controlRelayDaemonPublicKey = null;
  eventRelayDaemonPublicKey = null;
  constructor(endpoint, options = {}) {
    this.socketPath = endpoint;
    const staleMs = options.kernelEventStaleMs ?? DEFAULT_KERNEL_EVENT_STALE_MS;
    this.kernelEventStaleMs = staleMs > 0 ? Math.max(staleMs, 250) : 0;
    this.kernelPingIntervalMs = Math.max(options.kernelPingIntervalMs ?? DEFAULT_KERNEL_PING_INTERVAL_MS, 250);
    this.kernelMaxMissedPongs = Math.max(options.kernelMaxMissedPongs ?? DEFAULT_KERNEL_MAX_MISSED_PONGS, 1);
    this.relayAuthToken = options.relayAuthToken?.trim() || null;
    this.relayTarget = this.relayAuthToken ? {
      daemon_id: options.targetDaemonId?.trim() || null,
      daemon_alias: options.targetDaemonAlias?.trim() || null
    } : null;
  }
  supportsKernelEvents() {
    return isWebSocketEndpoint(this.socketPath);
  }
  isRelayMode() {
    return this.relayAuthToken != null;
  }
  send(request) {
    if (isWebSocketEndpoint(this.socketPath)) {
      return this.sendWebSocket(request);
    }
    return this.sendLocalSocket(request);
  }
  async subscribeToKernelEvents(sessionId, attachmentId) {
    if (!this.supportsKernelEvents()) {
      return;
    }
    const previousSubscription = this.activeKernelSubscription;
    const resumeFromEventId = previousSubscription?.sessionId === sessionId && previousSubscription.attachmentId === attachmentId ? this.lastReceivedEventId : null;
    if (resumeFromEventId == null) {
      this.lastReceivedEventId = null;
    }
    this.activeKernelSubscription = {
      sessionId,
      attachmentId,
      relaySubscriptionId: this.isRelayMode() ? randomUUID() : null,
      relayPrivateKey: null
    };
    try {
      if (this.isRelayMode()) {
        await this.sendRelaySubscribe(sessionId, attachmentId, resumeFromEventId);
      } else {
        await this.sendWebSocket({
          __kernel_transport: {
            type: "subscribe",
            session_id: sessionId,
            attachment_id: attachmentId,
            resume_from_event_id: resumeFromEventId
          }
        }, "event");
      }
      this.clearReconnectState();
      this.markKernelEventReceived();
    } catch (error) {
      this.scheduleReconnect();
      throw error;
    }
  }
  async unsubscribeFromKernelEvents() {
    if (!this.supportsKernelEvents()) {
      return;
    }
    const subscription = this.activeKernelSubscription;
    this.activeKernelSubscription = null;
    this.clearReconnectState();
    this.clearKernelEventWatchdog();
    const socket = this.getWebSocket("event");
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      return;
    }
    if (this.isRelayMode()) {
      if (!subscription?.relaySubscriptionId || !subscription.relayPrivateKey) {
        return;
      }
      await this.sendRelayUnsubscribe(subscription.relaySubscriptionId, subscription.relayPrivateKey);
    } else {
      await this.sendWebSocket({
        __kernel_transport: {
          type: "unsubscribe"
        }
      }, "event");
    }
  }
  async restartKernelEventStream() {
    if (!this.supportsKernelEvents() || !this.activeKernelSubscription) {
      return;
    }
    this.clearReconnectState();
    this.clearKernelEventWatchdog();
    this.clearKernelHeartbeat("event");
    const socket = this.getWebSocket("event");
    if (socket && socket.readyState !== WebSocket.CLOSED) {
      this.suppressNextEventCloseEvent = true;
      socket.terminate();
      this.setWebSocket("event", null);
      this.setWebSocketConnectPromise("event", null);
    }
    this.scheduleReconnect(25);
  }
  onKernelEvent(handler) {
    this.eventHandlers.add(handler);
    return () => {
      this.eventHandlers.delete(handler);
    };
  }
  async close() {
    this.activeKernelSubscription = null;
    this.clearReconnectState();
    this.clearKernelEventWatchdog();
    this.clearKernelHeartbeat("control");
    this.clearKernelHeartbeat("event");
    this.controlRelayDaemonPublicKey = null;
    this.eventRelayDaemonPublicKey = null;
    await Promise.all([this.closeWebSocket("control"), this.closeWebSocket("event")]);
  }
  sendLocalSocket(request) {
    return new Promise((resolve, reject) => {
      const socket = net.createConnection(this.socketPath);
      const chunks = [];
      let settled = false;
      const fail = (operation, error) => {
        if (settled) {
          return;
        }
        settled = true;
        socket.destroy();
        reject(new LocalIpcError(operation, error instanceof Error ? error.message : String(error)));
      };
      const succeed = value => {
        if (settled) {
          return;
        }
        settled = true;
        socket.destroy();
        resolve(value);
      };
      socket.setTimeout(IPC_TIMEOUT_MS);
      socket.once("timeout", () => fail("handle local response", "timed out"));
      socket.once("error", error => fail("connect local socket", error));
      socket.once("connect", () => {
        let payload;
        try {
          payload = Buffer.from(JSON.stringify(request), "utf8");
        } catch (error) {
          fail("serialize local request", error);
          return;
        }
        const frame = Buffer.allocUnsafe(4 + payload.length);
        frame.writeUInt32BE(payload.length, 0);
        payload.copy(frame, 4);
        socket.write(frame, error => {
          if (error) {
            fail("write local request", error);
          }
        });
      });
      socket.on("data", chunk => {
        chunks.push(chunk);
      });
      socket.once("end", () => {
        const buffer = Buffer.concat(chunks);
        if (buffer.length < 4) {
          fail("read local response header", "response header was truncated");
          return;
        }
        const payloadLength = buffer.readUInt32BE(0);
        const payload = buffer.subarray(4);
        if (payload.length < payloadLength) {
          fail("read local response body", "response body was truncated");
          return;
        }
        let envelope;
        try {
          envelope = JSON.parse(payload.subarray(0, payloadLength).toString("utf8"));
        } catch (error) {
          fail("decode local response", error);
          return;
        }
        if (envelope.error) {
          fail("handle local response", envelope.error);
          return;
        }
        if (envelope.response == null) {
          fail("handle local response", "response envelope was empty");
          return;
        }
        succeed(envelope.response);
      });
    });
  }
  async sendWebSocket(request, lane = "control") {
    const socket = await this.ensureWebSocket(lane);
    const requestId = randomUUID();
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(requestId);
        reject(new LocalIpcError("handle kernel response", "timed out", "request_timeout", true));
      }, IPC_TIMEOUT_MS);
      this.pending.set(requestId, {
        resolve: resolve,
        reject,
        timeout,
        relayPrivateKey: null,
        lane
      });
      try {
        const relayRequest = this.isRelayMode() ? normalizeRelayRequest(requestId, request, this.relayTarget, this.getRelayDaemonPublicKey(lane)) : null;
        if (relayRequest) {
          const pending = this.pending.get(requestId);
          if (pending) {
            pending.relayPrivateKey = relayRequest.privateKey;
          }
        }
        const payload = relayRequest ? relayRequest.frame : normalizeWebSocketRequest(requestId, request);
        socket.send(JSON.stringify(payload));
      } catch (error) {
        clearTimeout(timeout);
        this.pending.delete(requestId);
        reject(new LocalIpcError("write kernel request", error instanceof Error ? error.message : String(error), "write_failed", true));
      }
    });
  }
  async sendRelaySubscribe(sessionId, attachmentId, resumeFromEventId) {
    const lane = "event";
    const socket = await this.ensureWebSocket(lane);
    const requestId = randomUUID();
    const subscription = this.activeKernelSubscription;
    if (!subscription?.relaySubscriptionId) {
      throw new LocalIpcError("write relay subscribe", "relay subscription state is missing");
    }
    const subscriptionId = subscription.relaySubscriptionId;
    const keypair = createRelayKeypair();
    subscription.relayPrivateKey = keypair.privateKey;
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(requestId);
        reject(new LocalIpcError("handle kernel response", "timed out", "request_timeout", true));
      }, IPC_TIMEOUT_MS);
      this.pending.set(requestId, {
        resolve: () => resolve(),
        reject,
        timeout,
        relayPrivateKey: keypair.privateKey,
        lane
      });
      try {
        const frame = {
          kind: "client_subscribe",
          request_id: requestId,
          subscription_id: subscriptionId,
          target: requireRelayTarget(this.relayTarget),
          session_id: sessionId,
          attachment_id: attachmentId,
          client_public_key: keypair.publicKeyBase64,
          resume_from_event_id: resumeFromEventId
        };
        socket.send(JSON.stringify(frame));
      } catch (error) {
        clearTimeout(timeout);
        this.pending.delete(requestId);
        reject(new LocalIpcError("write relay subscribe", error instanceof Error ? error.message : String(error), "write_failed", true));
      }
    });
  }
  async sendRelayUnsubscribe(subscriptionId, privateKey) {
    const lane = "event";
    const socket = await this.ensureWebSocket(lane);
    const requestId = randomUUID();
    const publicKeyBase64 = relayPublicKeyFromPrivateKey(privateKey);
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(requestId);
        reject(new LocalIpcError("handle kernel response", "timed out", "request_timeout", true));
      }, IPC_TIMEOUT_MS);
      this.pending.set(requestId, {
        resolve: () => resolve(),
        reject,
        timeout,
        relayPrivateKey: privateKey,
        lane
      });
      try {
        const frame = {
          kind: "client_unsubscribe",
          request_id: requestId,
          subscription_id: subscriptionId,
          client_public_key: publicKeyBase64
        };
        socket.send(JSON.stringify(frame));
      } catch (error) {
        clearTimeout(timeout);
        this.pending.delete(requestId);
        reject(new LocalIpcError("write relay unsubscribe", error instanceof Error ? error.message : String(error), "write_failed", true));
      }
    });
  }
  async ensureWebSocket(lane = "control") {
    const existing = this.getWebSocket(lane);
    if (existing?.readyState === WebSocket.OPEN) {
      return existing;
    }
    const connectPromise = this.getWebSocketConnectPromise(lane);
    if (connectPromise) {
      return connectPromise;
    }
    const nextConnectPromise = new Promise((resolve, reject) => {
      const socket = new WebSocket(this.socketPath);
      let settled = false;
      const fail = (operation, error) => {
        if (settled) {
          return;
        }
        settled = true;
        this.setWebSocketConnectPromise(lane, null);
        reject(new LocalIpcError(operation, error instanceof Error ? error.message : String(error)));
      };
      socket.once("open", () => {
        const finalizeOpen = () => {
          settled = true;
          this.setWebSocket(lane, socket);
          this.setWebSocketConnectPromise(lane, null);
          this.setSuppressNextCloseEvent(lane, false);
          this.startKernelHeartbeat(socket, lane);
          socket.on("message", data => {
            this.handleWebSocketMessage(data, lane);
          });
          socket.on("pong", () => {
            this.setMissedKernelPongs(lane, 0);
          });
          socket.once("close", (code, reason) => {
            const suppressed = this.getSuppressNextCloseEvent(lane);
            this.setSuppressNextCloseEvent(lane, false);
            this.rejectPending("kernel websocket closed", lane);
            this.setWebSocket(lane, null);
            this.setRelayDaemonPublicKey(lane, null);
            this.clearKernelHeartbeat(lane);
            const closeMessage = reason.length > 0 ? reason.toString("utf8") : `kernel websocket closed${code ? ` (${code})` : ""}`;
            if (!suppressed) {
              this.emitSyntheticEvent({
                event: "transport_closed",
                message: closeMessage
              });
              if (lane === "event") {
                this.scheduleReconnect();
              }
            }
          });
          socket.once("error", error => {
            const suppressed = this.getSuppressNextCloseEvent(lane);
            this.setSuppressNextCloseEvent(lane, false);
            this.rejectPending(error.message, lane);
            this.setWebSocket(lane, null);
            this.setRelayDaemonPublicKey(lane, null);
            this.clearKernelHeartbeat(lane);
            if (!suppressed) {
              this.emitSyntheticEvent({
                event: "transport_closed",
                message: error.message
              });
              if (lane === "event") {
                this.scheduleReconnect();
              }
            }
          });
          resolve(socket);
        };
        if (!this.isRelayMode()) {
          finalizeOpen();
          return;
        }
        const handleRelayHandshakeMessage = data => {
          let frame;
          try {
            frame = JSON.parse(String(data));
          } catch (error) {
            fail("connect relay transport", error);
            return;
          }
          if (frame.kind === "client_connected") {
            if (!frame.daemon_public_key) {
              fail("connect relay transport", "relay did not provide daemon public key");
              return;
            }
            this.setRelayDaemonPublicKey(lane, frame.daemon_public_key);
            socket.off("message", handleRelayHandshakeMessage);
            finalizeOpen();
            return;
          }
          if (frame.kind === "close") {
            fail("connect relay transport", frame.reason);
            return;
          }
          fail("connect relay transport", "unexpected relay handshake frame");
        };
        socket.on("message", handleRelayHandshakeMessage);
        try {
          socket.send(JSON.stringify(buildRelayConnectFrame(this.relayAuthToken, this.relayTarget)));
        } catch (error) {
          socket.off("message", handleRelayHandshakeMessage);
          fail("write relay connect frame", error);
        }
      });
      socket.once("error", error => fail("connect kernel websocket", error));
    });
    this.setWebSocketConnectPromise(lane, nextConnectPromise);
    return nextConnectPromise;
  }
  handleWebSocketMessage(data, lane) {
    let frame;
    try {
      frame = JSON.parse(String(data));
    } catch (error) {
      this.rejectPending(error instanceof Error ? error.message : String(error), lane);
      return;
    }
    if ("type" in frame && frame.type === "event") {
      this.lastReceivedEventId = frame.event_id;
      this.markKernelEventReceived();
      for (const handler of this.eventHandlers) {
        handler(frame.event);
      }
      return;
    }
    if ("kind" in frame && frame.kind === "close") {
      this.rejectPending(frame.reason, lane);
      return;
    }
    if ("kind" in frame && frame.kind === "client_event") {
      const subscription = this.activeKernelSubscription;
      if (!subscription?.relayPrivateKey || subscription.relaySubscriptionId !== frame.subscription_id) {
        return;
      }
      try {
        const decrypted = decryptRelayPayload(subscription.relayPrivateKey, frame.encrypted_event);
        const event = JSON.parse(decrypted);
        this.lastReceivedEventId = frame.event_id;
        this.markKernelEventReceived();
        this.emitSyntheticEvent(event);
      } catch (error) {
        this.rejectPending(error instanceof Error ? error.message : String(error), lane);
      }
      return;
    }
    const requestId = "type" in frame ? frame.request_id : frame.request_id;
    const pending = this.pending.get(requestId);
    if (!pending) {
      return;
    }
    clearTimeout(pending.timeout);
    this.pending.delete(requestId);
    if (frame.error) {
      pending.reject(new LocalIpcError("handle kernel response", frame.error.message, frame.error.code, frame.error.retryable));
      return;
    }
    if ("kind" in frame) {
      if (!pending.relayPrivateKey) {
        pending.reject(new LocalIpcError("handle kernel response", "missing relay request key"));
        return;
      }
      if (frame.encrypted_response == null) {
        pending.reject(new LocalIpcError("handle kernel response", "response envelope was empty"));
        return;
      }
      try {
        const decrypted = decryptRelayPayload(pending.relayPrivateKey, frame.encrypted_response);
        pending.resolve(JSON.parse(decrypted));
      } catch (error) {
        pending.reject(new LocalIpcError("handle kernel response", error instanceof Error ? error.message : String(error)));
      }
      return;
    }
    if (frame.response == null) {
      pending.reject(new LocalIpcError("handle kernel response", "response envelope was empty"));
      return;
    }
    pending.resolve(frame.response);
  }
  rejectPending(message, lane) {
    const pendingEntries = Array.from(this.pending.entries()).filter(([, pending]) => !lane || pending.lane === lane);
    for (const [requestId] of pendingEntries) {
      this.pending.delete(requestId);
    }
    for (const [, pending] of pendingEntries) {
      clearTimeout(pending.timeout);
      pending.reject(new LocalIpcError("kernel websocket", message, "connection_closed", true));
    }
  }
  emitSyntheticEvent(event) {
    for (const handler of this.eventHandlers) {
      handler(event);
    }
  }
  clearReconnectState() {
    if (this.reconnectTimeout) {
      clearTimeout(this.reconnectTimeout);
      this.reconnectTimeout = null;
    }
    this.reconnectDelayMs = 250;
  }
  markKernelEventReceived() {
    this.lastKernelEventAtMs = Date.now();
    this.armKernelEventWatchdog();
  }
  armKernelEventWatchdog() {
    this.clearKernelEventWatchdog();
    if (!this.kernelEventStaleMs || !this.activeKernelSubscription || this.eventHandlers.size === 0) {
      return;
    }
    this.kernelEventWatchdog = setTimeout(() => {
      const elapsedMs = Date.now() - this.lastKernelEventAtMs;
      if (!this.activeKernelSubscription || this.eventHandlers.size === 0) {
        return;
      }
      if (elapsedMs < this.kernelEventStaleMs) {
        this.armKernelEventWatchdog();
        return;
      }
      this.emitSyntheticEvent({
        event: "transport_closed",
        message: `kernel event stream stalled for ${elapsedMs}ms; reconnecting`
      });
      void this.restartKernelEventStream();
    }, this.kernelEventStaleMs);
  }
  clearKernelEventWatchdog() {
    if (this.kernelEventWatchdog) {
      clearTimeout(this.kernelEventWatchdog);
      this.kernelEventWatchdog = null;
    }
  }
  startKernelHeartbeat(socket, lane) {
    this.clearKernelHeartbeat(lane);
    this.setMissedKernelPongs(lane, 0);
    const heartbeat = setInterval(() => {
      if (socket !== this.getWebSocket(lane) || socket.readyState !== WebSocket.OPEN) {
        this.clearKernelHeartbeat(lane);
        return;
      }
      if (this.getMissedKernelPongs(lane) >= this.kernelMaxMissedPongs) {
        if (lane === "event") {
          this.emitSyntheticEvent({
            event: "transport_closed",
            message: "kernel websocket heartbeat missed; reconnecting"
          });
        }
        this.setSuppressNextCloseEvent(lane, true);
        socket.terminate();
        this.setWebSocket(lane, null);
        this.setRelayDaemonPublicKey(lane, null);
        if (lane === "event") {
          this.scheduleReconnect();
        }
        return;
      }
      this.setMissedKernelPongs(lane, this.getMissedKernelPongs(lane) + 1);
      try {
        socket.ping();
      } catch {
        if (lane === "event") {
          this.emitSyntheticEvent({
            event: "transport_closed",
            message: "kernel websocket heartbeat failed; reconnecting"
          });
        }
        this.setSuppressNextCloseEvent(lane, true);
        socket.terminate();
        this.setWebSocket(lane, null);
        this.setRelayDaemonPublicKey(lane, null);
        if (lane === "event") {
          this.scheduleReconnect();
        }
      }
    }, this.kernelPingIntervalMs);
    if (lane === "control") {
      this.controlHeartbeat = heartbeat;
    } else {
      this.eventHeartbeat = heartbeat;
    }
  }
  clearKernelHeartbeat(lane) {
    const heartbeat = lane === "control" ? this.controlHeartbeat : this.eventHeartbeat;
    if (heartbeat) {
      clearInterval(heartbeat);
      if (lane === "control") {
        this.controlHeartbeat = null;
      } else {
        this.eventHeartbeat = null;
      }
    }
    this.setMissedKernelPongs(lane, 0);
  }
  scheduleReconnect(delayMs = this.reconnectDelayMs) {
    if (!this.activeKernelSubscription || this.eventHandlers.size === 0 || this.reconnectTimeout) {
      return;
    }
    this.reconnectTimeout = setTimeout(() => {
      this.reconnectTimeout = null;
      void this.resumeKernelSubscription();
    }, delayMs);
    this.reconnectDelayMs = Math.min(Math.max(delayMs * 2, 250), 5_000);
  }
  async resumeKernelSubscription() {
    const subscription = this.activeKernelSubscription;
    if (!subscription || this.eventHandlers.size === 0) {
      return;
    }
    try {
      if (this.isRelayMode()) {
        await this.sendRelaySubscribe(subscription.sessionId, subscription.attachmentId, this.lastReceivedEventId);
      } else {
        await this.sendWebSocket({
          __kernel_transport: {
            type: "subscribe",
            session_id: subscription.sessionId,
            attachment_id: subscription.attachmentId,
            resume_from_event_id: this.lastReceivedEventId
          }
        }, "event");
      }
      this.clearReconnectState();
      this.markKernelEventReceived();
      this.emitSyntheticEvent({
        event: "transport_resumed",
        session_id: subscription.sessionId,
        resumed_from_event_id: this.lastReceivedEventId
      });
    } catch {
      this.scheduleReconnect();
    }
  }
  getWebSocket(lane) {
    return lane === "control" ? this.controlWebsocket : this.eventWebsocket;
  }
  setWebSocket(lane, socket) {
    if (lane === "control") {
      this.controlWebsocket = socket;
    } else {
      this.eventWebsocket = socket;
    }
  }
  getWebSocketConnectPromise(lane) {
    return lane === "control" ? this.controlWebsocketConnectPromise : this.eventWebsocketConnectPromise;
  }
  setWebSocketConnectPromise(lane, promise) {
    if (lane === "control") {
      this.controlWebsocketConnectPromise = promise;
    } else {
      this.eventWebsocketConnectPromise = promise;
    }
  }
  getRelayDaemonPublicKey(lane) {
    return lane === "control" ? this.controlRelayDaemonPublicKey : this.eventRelayDaemonPublicKey;
  }
  setRelayDaemonPublicKey(lane, publicKey) {
    if (lane === "control") {
      this.controlRelayDaemonPublicKey = publicKey;
    } else {
      this.eventRelayDaemonPublicKey = publicKey;
    }
  }
  getSuppressNextCloseEvent(lane) {
    return lane === "control" ? this.suppressNextControlCloseEvent : this.suppressNextEventCloseEvent;
  }
  setSuppressNextCloseEvent(lane, value) {
    if (lane === "control") {
      this.suppressNextControlCloseEvent = value;
    } else {
      this.suppressNextEventCloseEvent = value;
    }
  }
  getMissedKernelPongs(lane) {
    return lane === "control" ? this.missedControlPongs : this.missedEventPongs;
  }
  setMissedKernelPongs(lane, value) {
    if (lane === "control") {
      this.missedControlPongs = value;
    } else {
      this.missedEventPongs = value;
    }
  }
  async closeWebSocket(lane) {
    const socket = this.getWebSocket(lane);
    this.setWebSocket(lane, null);
    this.setWebSocketConnectPromise(lane, null);
    this.setRelayDaemonPublicKey(lane, null);
    if (!socket || socket.readyState === WebSocket.CLOSED) {
      return;
    }
    await new Promise(resolve => {
      this.setSuppressNextCloseEvent(lane, true);
      socket.once("close", () => resolve());
      socket.close();
    });
  }
}
function buildRelayConnectFrame(authToken, target) {
  if (!authToken) {
    throw new Error("relay auth token is required");
  }
  if (!target?.daemon_id && !target?.daemon_alias) {
    throw new Error("relay target daemon id or alias is required");
  }
  return {
    kind: "client_connect",
    auth_token: authToken,
    target: target ?? {}
  };
}
function normalizeRelayRequest(requestId, request, target, daemonPublicKey) {
  const resolvedTarget = requireRelayTarget(target);
  if (!daemonPublicKey) {
    throw new Error("relay daemon public key is required");
  }
  const plaintext = Buffer.from(JSON.stringify(request), "utf8");
  const {
    privateKey,
    payload
  } = encryptRelayPayload(daemonPublicKey, plaintext);
  return {
    frame: {
      kind: "client_request",
      request_id: requestId,
      target: resolvedTarget,
      encrypted_request: payload
    },
    privateKey
  };
}
function isWebSocketEndpoint(value) {
  return value.startsWith("ws://") || value.startsWith("wss://");
}
function normalizeWebSocketRequest(requestId, request) {
  const transportRequest = extractTransportRequest(request);
  if (transportRequest?.type === "subscribe") {
    return {
      type: "subscribe",
      request_id: requestId,
      session_id: transportRequest.session_id,
      attachment_id: transportRequest.attachment_id,
      resume_from_event_id: transportRequest.resume_from_event_id ?? null
    };
  }
  if (transportRequest?.type === "unsubscribe") {
    return {
      type: "unsubscribe",
      request_id: requestId
    };
  }
  return {
    type: "request",
    request_id: requestId,
    request
  };
}
function extractTransportRequest(request) {
  if (!request || typeof request !== "object") {
    return null;
  }
  const value = request.__kernel_transport;
  if (!value || typeof value !== "object") {
    return null;
  }
  const transport = value;
  if (transport.type === "subscribe" && typeof transport.session_id === "string" && typeof transport.attachment_id === "string") {
    return {
      type: "subscribe",
      session_id: transport.session_id,
      attachment_id: transport.attachment_id,
      resume_from_event_id: typeof transport.resume_from_event_id === "number" ? transport.resume_from_event_id : null
    };
  }
  if (transport.type === "unsubscribe") {
    return {
      type: "unsubscribe"
    };
  }
  return null;
}
const RELAY_NONCE_LEN = 12;
const RELAY_TAG_LEN = 16;
const RELAY_INFO = Buffer.from("arroba-relay-v1", "utf8");
function encryptRelayPayload(peerPublicKeyBase64, plaintext) {
  const ecdh = createECDH("prime256v1");
  const publicKey = ecdh.generateKeys();
  const privateKey = ecdh.getPrivateKey();
  const sharedSecret = ecdh.computeSecret(Buffer.from(peerPublicKeyBase64, "base64"));
  const key = deriveRelayKey(sharedSecret);
  const nonce = randomBytes(RELAY_NONCE_LEN);
  const cipher = createCipheriv("aes-256-gcm", key, nonce);
  const ciphertext = Buffer.concat([cipher.update(plaintext), cipher.final(), cipher.getAuthTag()]);
  return {
    privateKey,
    payload: {
      sender_public_key: publicKey.toString("base64"),
      nonce: nonce.toString("base64"),
      ciphertext: ciphertext.toString("base64")
    }
  };
}
function decryptRelayPayload(privateKey, payload) {
  const ecdh = createECDH("prime256v1");
  ecdh.setPrivateKey(privateKey);
  const sharedSecret = ecdh.computeSecret(Buffer.from(payload.sender_public_key, "base64"));
  const key = deriveRelayKey(sharedSecret);
  const nonce = Buffer.from(payload.nonce, "base64");
  if (nonce.length !== RELAY_NONCE_LEN) {
    throw new Error("invalid relay nonce");
  }
  const ciphertext = Buffer.from(payload.ciphertext, "base64");
  if (ciphertext.length < RELAY_TAG_LEN) {
    throw new Error("invalid relay ciphertext");
  }
  const body = ciphertext.subarray(0, ciphertext.length - RELAY_TAG_LEN);
  const tag = ciphertext.subarray(ciphertext.length - RELAY_TAG_LEN);
  const decipher = createDecipheriv("aes-256-gcm", key, nonce);
  decipher.setAuthTag(tag);
  const plaintext = Buffer.concat([decipher.update(body), decipher.final()]);
  return plaintext.toString("utf8");
}
function deriveRelayKey(sharedSecret) {
  return Buffer.from(hkdfSync("sha256", sharedSecret, Buffer.alloc(0), RELAY_INFO, 32));
}
function createRelayKeypair() {
  const ecdh = createECDH("prime256v1");
  const publicKey = ecdh.generateKeys();
  return {
    privateKey: ecdh.getPrivateKey(),
    publicKeyBase64: publicKey.toString("base64")
  };
}
function relayPublicKeyFromPrivateKey(privateKey) {
  const ecdh = createECDH("prime256v1");
  ecdh.setPrivateKey(privateKey);
  return ecdh.getPublicKey().toString("base64");
}
function requireRelayTarget(target) {
  if (!target?.daemon_id && !target?.daemon_alias) {
    throw new Error("relay target daemon id or alias is required");
  }
  return target;
}