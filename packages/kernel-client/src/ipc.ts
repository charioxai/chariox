import { TerminalLocalDirect } from "./terminal-local-direct.js"
import { relayAuthorization, requireRenewedRelayAuthorization, RelayAuthorizationRenewal, reauthenticateRelaySocket, relayCloseError, relayAuthorizationRenewalCapability, relayAuthorizationRenewalMinimumProtocolVersion } from "./relay-authorization.js"
import { issueCloudRelayClientTokenRequest } from "./ipc-relay-control-requests.js"
import { isLocalRelayIssuerEndpoint, type RelayAuthorizationIssuer } from "./relay-authorization.js"
import { randomUUID } from "node:crypto"
import {
  closeSync,
  constants as fsConstants,
  fstatSync,
  lstatSync,
  openSync,
  readFileSync,
  unlinkSync,
} from "node:fs"

import WebSocket from "ws"

import { kernelUpgradeRejectionHandler } from "./websocket-upgrade-rejection.js"

import { getKernelResourceTelemetryRequest } from "./ipc-kernel-control-requests.js"
import { requireKernelControlCapability } from "./ipc-disposable-worker-requests.js"
import type { KernelEvent } from "./kernel-events.js"
import type {
  IpcEnvelope,
  KernelSocketLane,
  KernelTransportEventFrame,
  KernelTransportResponseFrame,
  RelayCloseFrame,
  RelayConnectedFrame,
  RelayEventFrame,
  RelayResponseFrame,
  RelayTarget,
} from "./kernel-transport-frames.js"
import type { KernelResourceTelemetryResponse } from "./kernel-types.js"
import { normalizeWebSocketRequest } from "./kernel-transport-requests.js"
import {
  buildKernelSubscriptionTransportRequest,
  createKernelSessionSubscriptionStart,
  createWaitingRoomInventorySubscriptionStart,
  kernelSubscriptionScopeValue,
  type KernelSubscriptionState,
} from "./kernel-subscriptions.js"
import { readLocalKernelAuthToken } from "./local-kernel-auth-token.js"
import { LocalIpcError } from "./local-ipc-error.js"
import { waitsForKernelAuthorization } from "./kernel-authorization-request-policy.js"
import {
  createRelayKeypair,
  type RelayClientIdentity,
} from "./relay-crypto.js"
import type { EncryptedRelayPayload } from "./kernel-transport-frames.js"
import {
  buildRelayConnectFrame,
  buildRelaySubscribeFrame,
  buildRelayUnsubscribeFrame,
  decryptRelayPayloadFromExpectedSender,
  normalizeRelayRequest,
} from "./relay-transport.js"
import { KernelPendingRequestRegistry } from "./websocket-pending-requests.js"
import { KernelRequestLifetime, waitForKernelRequestReplay } from "./websocket-request-lifetime.js"
import { formatTransportError, isWebSocketEndpoint } from "./websocket-transport-diagnostics.js"
import { KernelSocketResponsiveness } from "./kernel-socket-responsiveness.js"

// Slice start can cold-build the managed Linux image before returning the
// worker kernel endpoint. Keep the control request open long enough for first
// run provisioning while lifecycle progress remains request/response based.
const IPC_TIMEOUT_MS = 600_000
const DEFAULT_KERNEL_EVENT_STALE_MS = 0
const DEFAULT_KERNEL_PING_INTERVAL_MS = 5_000
const DEFAULT_KERNEL_MAX_MISSED_PONGS = 2
const IPC_WEBSOCKET_CLOSE_TIMEOUT_MS = 1_000
const IPC_CLIENT_CLOSE_TIMEOUT_MS = 1_500
const KERNEL_RECONNECT_BASE_DELAY_MS = 250
const KERNEL_RECONNECT_MAX_DELAY_MS = 5_000
const KERNEL_RECONNECT_JITTER_MS = 250
const KERNEL_CONTROL_REQUEST_RETRY_DEADLINE_MS = 60_000
const KERNEL_CONTROL_RESPONSE_STALL_MS = 5_000
// The kernel runs these again when they are replayed: they carry no request id
// its ledgers deduplicate, and it keeps them out of its command-result cache
// (`request_is_cacheable`, whose tests check this list). Each stops an App
// worker, which can outlast the stall window; a replay then meets the first
// one's operation guard and answers `busy` (or, once the first has finished,
// restarts the worker again or is refused by the uninstall's generation fence)
// although the first one succeeds. Once written, they wait for their answer
// and are never resent; losing the answer rejects with `outcome_unknown`.
const KERNEL_REQUESTS_RUN_AGAIN_ON_REPLAY = new Set(["ControlAppWorker", "UninstallApp"])
const MAX_KERNEL_LOCAL_AUTH_TOKEN_BYTES = 8 * 1024

export type { KernelEvent } from "./kernel-events.js"
export { LocalIpcError } from "./local-ipc-error.js"
export { RelayClientIdentity } from "./relay-crypto.js"

type BoundKernelLocalAuthCredential = {
  endpoint: string
  token: string
}

const hostedPublicationEnvironmentNames = [
  "CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL",
  "CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL_FILE",
  "CHARIOX_PUBLICATION_CLOUD_API_URL",
  "CHARIOX_PUBLICATION_CLOUD_DEPLOYMENT_ID",
  "CHARIOX_PUBLICATION_CLOUD_RUNNER_KEY",
] as const

let kernelLocalAuthCredentialFromEnvironment: BoundKernelLocalAuthCredential | undefined

export function consumeKernelLocalAuthTokenFromEnv(endpoint = configuredLocalKernelEndpoint()): string | undefined {
  const rawEnvironmentToken = process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN
  const rawTokenFile = process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE
  delete process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN
  delete process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE
  if (rawEnvironmentToken !== undefined && rawTokenFile !== undefined) {
    throw new Error("kernel local auth token and token file cannot both be configured")
  }
  if (kernelLocalAuthCredentialFromEnvironment) {
    if (rawEnvironmentToken !== undefined || rawTokenFile !== undefined) {
      throw new Error("kernel local auth credential cannot be reconfigured after consumption")
    }
    const canonicalEndpoint = requireCanonicalLoopbackKernelEndpoint(endpoint)
    if (canonicalEndpoint !== kernelLocalAuthCredentialFromEnvironment.endpoint) {
      throw new Error(
        `kernel local auth credential is bound to kernel endpoint ${kernelLocalAuthCredentialFromEnvironment.endpoint}`,
      )
    }
    return kernelLocalAuthCredentialFromEnvironment.token
  }
  if (rawEnvironmentToken === undefined && rawTokenFile === undefined) return undefined

  const canonicalEndpoint = requireCanonicalLoopbackKernelEndpoint(endpoint)
  const environmentToken = rawEnvironmentToken?.trim()
  const tokenFile = rawTokenFile?.trim()
  if (rawEnvironmentToken !== undefined && !environmentToken) {
    throw new Error("kernel local auth token must not be empty")
  }
  if (rawTokenFile !== undefined && !tokenFile) {
    throw new Error("kernel local auth token file path must not be empty")
  }
  if (environmentToken && isHostedPublicationGateway()) {
    throw new Error("hosted publication gateways require a one-shot kernel local auth token file")
  }
  const token = environmentToken ?? readPrivateKernelLocalAuthToken(tokenFile!)
  kernelLocalAuthCredentialFromEnvironment = { endpoint: canonicalEndpoint, token }
  return token
}

function readPrivateKernelLocalAuthToken(path: string): string {
  let descriptor: number
  try {
    descriptor = openSync(path, fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW)
  } catch (error) {
    throw new Error(`kernel local auth token file could not be opened safely: ${String(error)}`)
  }
  try {
    const metadata = fstatSync(descriptor)
    const currentUid = process.getuid?.()
    if (
      !metadata.isFile()
      || (metadata.mode & 0o077) !== 0
      || (currentUid !== undefined && metadata.uid !== currentUid)
      || metadata.nlink !== 1
      || metadata.size > MAX_KERNEL_LOCAL_AUTH_TOKEN_BYTES
    ) {
      throw new Error(
        "kernel local auth token file must be a bounded, single-link owned regular file with mode 0600",
      )
    }
    const pathMetadata = lstatSync(path)
    if (
      !pathMetadata.isFile()
      || pathMetadata.isSymbolicLink()
      || pathMetadata.dev !== metadata.dev
      || pathMetadata.ino !== metadata.ino
    ) {
      throw new Error("kernel local auth token file changed while it was being consumed")
    }
    unlinkSync(path)
    if (fstatSync(descriptor).nlink !== 0) {
      throw new Error("kernel local auth token file was not consumed from its validated descriptor")
    }
    const token = readFileSync(descriptor, "utf8").trim()
    if (!token) throw new Error("kernel local auth token file must not be empty")
    return token
  } finally {
    closeSync(descriptor)
  }
}

function requireCanonicalLoopbackKernelEndpoint(endpoint: string): string {
  const canonicalEndpoint = canonicalLoopbackKernelEndpoint(endpoint)
  if (!canonicalEndpoint) {
    throw new Error("kernel local auth credentials require an exact canonical loopback kernel endpoint")
  }
  return canonicalEndpoint
}

function canonicalLoopbackKernelEndpoint(endpoint: string): string | null {
  let url: URL
  try {
    url = new URL(endpoint)
  } catch {
    return null
  }
  if (
    url.protocol !== "ws:"
    || (url.hostname !== "127.0.0.1" && url.hostname !== "[::1]")
    || url.username !== ""
    || url.password !== ""
    || url.pathname !== "/"
    || url.search !== ""
    || url.hash !== ""
  ) {
    return null
  }
  return url.href
}

function configuredLocalKernelEndpoint() {
  return process.env.CHARIOX_KERNEL_URL?.trim()
    || `ws://${process.env.CHARIOX_KERNEL_HOST?.trim() || "127.0.0.1"}:${process.env.CHARIOX_KERNEL_PORT?.trim() || "43118"}`
}

function isHostedPublicationGateway() {
  return hostedPublicationEnvironmentNames.some((name) => Boolean(process.env[name]?.trim()))
}

/** MP-08/MP-10: fixed transport metadata only; never auth or packet contents. */
export type KernelTransportDiagnostic = {
  lane: "control" | "event"
  cause: "reset" | "request_replay" | "heartbeat_missed" | "heartbeat_failed" | "socket_close" | "socket_error" | "lease_response_refused" | "lease_transport_failed"
    | "renewal_failed" | "authorization_ended"
  local: boolean
  missedPongs: number
  operation?: string
  code?: string | null
  retryable?: boolean
  closeCode?: number
  retrying?: boolean
  sequenceMatches?: boolean
  expired?: boolean
}

export type LocalIpcClientOptions = {
  onTransportDiagnostic?: ((diagnostic: KernelTransportDiagnostic) => void) | undefined
  /** State directory of a private local kernel; never used for relay connections. */
  localAuthEnvironment?: NodeJS.ProcessEnv | undefined
  localAuthToken?: string | undefined
  relayAuthToken?: string | undefined
  targetDaemonId?: string | undefined
  targetDaemonAlias?: string | undefined
  kernelEventStaleMs?: number | undefined
  kernelPingIntervalMs?: number | undefined
  kernelMaxMissedPongs?: number | undefined
  reconnectJitterMs?: number | undefined
  reconnectRandom?: (() => number) | undefined
  controlRequestRetryDeadlineMs?: number | undefined
  controlResponseStallMs?: number | undefined
  relayIdentity?: RelayClientIdentity | undefined
  relayAuthorizationIssuer?: RelayAuthorizationIssuer | undefined
}

export class LocalIpcClient {
  private readonly transportObserver: LocalIpcClientOptions["onTransportDiagnostic"]
  readonly socketPath: string
  private readonly relayIssuer: LocalIpcClient | null
  private readonly relayIssuerDaemonId: string | null
  private relayIssuerNoticeSent = false
  private readonly localAuthEnvironment: NodeJS.ProcessEnv
  private readonly localAuthEndpoint: string | null
  private readonly localAuthToken: string | null
  private relayAuthToken: string | null
  private readonly relayTarget: RelayTarget | null
  private readonly relayIdentity: RelayClientIdentity | null
  private readonly terminalLocalDirect: TerminalLocalDirect | null
  private relayRenewal: RelayAuthorizationRenewal | null = null
  private relayRenewalNegotiated = false
  private relayAuthorizationFailure: LocalIpcError | null = null
  private controlWebsocket: WebSocket | null = null
  private eventWebsocket: WebSocket | null = null
  private connectingControlWebsocket: WebSocket | null = null
  private connectingEventWebsocket: WebSocket | null = null
  private controlWebsocketConnectPromise: Promise<WebSocket> | null = null
  private eventWebsocketConnectPromise: Promise<WebSocket> | null = null
  private readonly pendingRequests = new KernelPendingRequestRegistry(IPC_TIMEOUT_MS)
  private readonly requestLifetime = new KernelRequestLifetime()
  private eventHandlers = new Set<(event: KernelEvent) => void>()
  private activeKernelSubscription: KernelSubscriptionState | null = null
  private reconnectTimeout: NodeJS.Timeout | null = null
  private reconnectDelayMs = 250
  private lastReceivedEventId: number | null = null
  private lastKernelEventAtMs = 0
  private kernelEventWatchdog: NodeJS.Timeout | null = null
  private controlHeartbeat: NodeJS.Timeout | null = null
  private eventHeartbeat: NodeJS.Timeout | null = null
  private readonly reconnectJitterMs: number
  private readonly reconnectRandom: () => number
  private readonly controlRequestRetryDeadlineMs: number
  private readonly controlResponseStallMs: number
  private missedControlPongs = 0
  private readonly socketResponsiveness = new KernelSocketResponsiveness()
  private missedEventPongs = 0
  private suppressNextControlCloseEvent = false
  private suppressNextEventCloseEvent = false
  private controlRelayDaemonPublicKey: string | null = null
  private eventRelayDaemonPublicKey: string | null = null
  private readonly kernelEventStaleMs: number
  private readonly kernelPingIntervalMs: number
  private readonly kernelMaxMissedPongs: number

  constructor(endpoint: string, options: LocalIpcClientOptions = {}) {
    this.transportObserver = options.onTransportDiagnostic
    if (!isWebSocketEndpoint(endpoint)) {
      if (endpoint.includes("://") && !endpoint.startsWith("unix://")) throw new Error("unsupported kernel endpoint")
      endpoint = `ws+unix://${endpoint.replace(/^unix:\/\//, "")}`
    }
    if (endpoint.startsWith("ws+unix://") && (options.localAuthToken !== undefined || options.relayAuthToken !== undefined)) {
      throw new Error("Unix kernel access uses OS process identity, without bearer credentials")
    }
    this.socketPath = endpoint
    const localAuthEnvironment = options.localAuthEnvironment ?? process.env
    this.localAuthEnvironment = {
      CHARIOX_HOME: localAuthEnvironment.CHARIOX_HOME,
      XDG_STATE_HOME: localAuthEnvironment.XDG_STATE_HOME,
      HOME: localAuthEnvironment.HOME,
    }
    const staleMs = options.kernelEventStaleMs ?? DEFAULT_KERNEL_EVENT_STALE_MS
    this.kernelEventStaleMs = staleMs > 0 ? Math.max(staleMs, 250) : 0
    this.kernelPingIntervalMs = Math.max(options.kernelPingIntervalMs ?? DEFAULT_KERNEL_PING_INTERVAL_MS, 250)
    this.kernelMaxMissedPongs = Math.max(options.kernelMaxMissedPongs ?? DEFAULT_KERNEL_MAX_MISSED_PONGS, 1)
    this.reconnectJitterMs = Math.max(options.reconnectJitterMs ?? KERNEL_RECONNECT_JITTER_MS, 0)
    this.reconnectRandom = options.reconnectRandom ?? Math.random
    this.controlRequestRetryDeadlineMs = Math.max(
      options.controlRequestRetryDeadlineMs ?? KERNEL_CONTROL_REQUEST_RETRY_DEADLINE_MS,
      0,
    )
    this.controlResponseStallMs = Math.max(
      options.controlResponseStallMs ?? KERNEL_CONTROL_RESPONSE_STALL_MS,
      10,
    )
    this.relayAuthToken = options.relayAuthToken?.trim() || null
    this.relayIdentity = options.relayIdentity ?? null
    const issuer = options.relayAuthorizationIssuer
    if (issuer && (!this.relayAuthToken || !issuer.daemonId || !isLocalRelayIssuerEndpoint(issuer.endpoint))) throw new Error("relay renewal issuer must identify an authenticated local kernel")
    this.relayIssuerDaemonId = issuer?.daemonId ?? null
    this.relayIssuer = issuer ? new LocalIpcClient(issuer.endpoint, {localAuthEnvironment, kernelPingIntervalMs: 60_000, controlRequestRetryDeadlineMs: 0, controlResponseStallMs: 10_000}) : null
    if (this.relayIdentity && !this.relayAuthToken) {
      throw new Error("a CLI relay identity can only be used with relay transport")
    }
    const explicitLocalAuthToken = options.localAuthToken?.trim()
    if (options.localAuthToken !== undefined && !explicitLocalAuthToken) {
      throw new Error("kernel local auth token must not be empty")
    }
    if (explicitLocalAuthToken && (
      process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN !== undefined
      || process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE !== undefined
    )) {
      throw new Error("explicit and environment kernel local auth credentials cannot both be configured")
    }
    if (this.relayAuthToken && (
      explicitLocalAuthToken
      || process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN !== undefined
      || process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE !== undefined
    )) {
      throw new Error("kernel local auth credentials cannot be used with relay transport")
    }
    if (explicitLocalAuthToken && isHostedPublicationGateway()) {
      throw new Error("hosted publication gateways require a one-shot kernel local auth token file")
    }
    this.localAuthToken = this.relayAuthToken || endpoint.startsWith("ws+unix://")
      ? null
      : explicitLocalAuthToken ?? consumeKernelLocalAuthTokenFromEnv(endpoint) ?? null
    this.localAuthEndpoint = this.localAuthToken
      ? requireCanonicalLoopbackKernelEndpoint(endpoint)
      : null
    this.relayTarget = this.relayAuthToken
      ? {
        daemon_id: options.targetDaemonId?.trim() || null,
        daemon_alias: options.targetDaemonAlias?.trim() || null,
      }
      : null
    this.terminalLocalDirect = this.relayAuthToken && this.relayTarget && this.relayIdentity
      ? new TerminalLocalDirect({ relayUrl: this.socketPath, token: () => this.relayAuthToken!, target: this.relayTarget,
        onDiagnostic: diagnostic => this.reportTransportDiagnostic("control", diagnostic.cause, diagnostic),
        identity: this.relayIdentity, eligible: () => this.localDirectEligible(this.relayTarget!),
        retryCarrier: () => {
          for (const lane of ["control", "event"] as const) {
            const socket = this.getWebSocket(lane)
            if (socket && !this.terminalLocalDirect?.isLocal(socket)) this.destroyWebSocket(lane, "retrying local terminal carrier")
          }
          this.scheduleReconnect()
        },
      }) : null
  }

  protected localDirectEligible(_target: RelayTarget): boolean { return false }

  isLocalDirectTransport(): boolean {
    return this.terminalLocalDirect?.isLocal(this.controlWebsocket) === true
  }

  supportsKernelEvents() {
    return isWebSocketEndpoint(this.socketPath)
  }

  isRelayTransport() {
    return this.isRelayMode()
  }

  getRelayClientIdentity(): RelayClientIdentity | null {
    return this.relayIdentity
  }

  private isRelayMode() {
    return this.relayAuthToken != null
  }

  async send<TResponse>(request: unknown): Promise<TResponse> {
    let admittedSocket: WebSocket | undefined
    await requireKernelControlCapability(async query => {
      admittedSocket = await this.ensureWebSocket("control")
      return this.sendWebSocket(query, "control", admittedSocket)
    }, request)
    if (admittedSocket) return this.sendWebSocket<TResponse>(request, "control", admittedSocket)
    return this.sendUnchecked<TResponse>(request)
  }

  private sendUnchecked<TResponse>(request: unknown): Promise<TResponse> {
    return this.sendWebSocket(request)
  }

  async getManagedTargetResourceTelemetry(options: {
    kernelRef?: string | null
    machineRef?: string | null
  } = {}): Promise<KernelResourceTelemetryResponse> {
    return this.send<KernelResourceTelemetryResponse>(getKernelResourceTelemetryRequest(options))
  }

  async subscribeToKernelEvents(sessionId: string, attachmentId: string): Promise<void> {
    if (!this.supportsKernelEvents()) {
      return
    }
    const start = createKernelSessionSubscriptionStart({
      previous: this.activeKernelSubscription,
      lastReceivedEventId: this.lastReceivedEventId,
      sessionId,
      attachmentId,
      relaySubscriptionId: this.isRelayMode() ? randomUUID() : null,
    })
    if (start.resetLastReceivedEventId) {
      this.lastReceivedEventId = null
    }
    this.activeKernelSubscription = start.subscription
    try {
      if (this.isRelayMode()) {
        await this.sendRelaySubscribe(start.subscription, start.resumeFromEventId)
      } else {
        await this.sendWebSocket<Record<string, unknown>>(
          buildKernelSubscriptionTransportRequest(start.subscription, start.resumeFromEventId),
          "event",
        )
      }
      this.clearReconnectState()
      this.markKernelEventReceived()
    } catch (error) {
      this.scheduleReconnect()
      throw error
    }
  }

  async subscribeToWaitingRoomInventory(): Promise<void> {
    if (!this.supportsKernelEvents()) {
      return
    }
    const start = createWaitingRoomInventorySubscriptionStart({
      previous: this.activeKernelSubscription,
      lastReceivedEventId: this.lastReceivedEventId,
      relaySubscriptionId: this.isRelayMode() ? randomUUID() : null,
    })
    if (start.resetLastReceivedEventId) {
      this.lastReceivedEventId = null
    }
    this.activeKernelSubscription = start.subscription
    try {
      if (this.isRelayMode()) {
        await this.sendRelaySubscribe(start.subscription, start.resumeFromEventId)
      } else {
        await this.sendWebSocket<Record<string, unknown>>(
          buildKernelSubscriptionTransportRequest(start.subscription, start.resumeFromEventId),
          "event",
        )
      }
      this.clearReconnectState()
      this.markKernelEventReceived()
    } catch (error) {
      this.scheduleReconnect()
      throw error
    }
  }

  async unsubscribeFromKernelEvents(): Promise<void> {
    if (!this.supportsKernelEvents()) {
      return
    }
    const subscription = this.activeKernelSubscription
    this.activeKernelSubscription = null
    this.clearReconnectState()
    this.clearKernelEventWatchdog()
    const socket = this.getWebSocket("event")
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      return
    }
    if (this.isRelayMode()) {
      if (!subscription?.relaySubscriptionId || !subscription.relayPublicKey) {
        return
      }
      if (!subscription.relayDecryptEvent) return
      await this.sendRelayUnsubscribe(
        subscription.relaySubscriptionId,
        subscription.relayPublicKey,
        subscription.relayDecryptEvent,
      )
    } else {
      await this.sendWebSocket<Record<string, unknown>>({
        __kernel_transport: {
          type: "unsubscribe",
        },
      }, "event")
    }
  }

  async restartKernelEventStream(): Promise<void> {
    if (!this.supportsKernelEvents() || !this.activeKernelSubscription) {
      return
    }
    this.clearReconnectState()
    this.clearKernelEventWatchdog()
    this.clearKernelHeartbeat("event")
    const socket = this.getWebSocket("event")
    if (socket && socket.readyState !== WebSocket.CLOSED) {
      this.suppressNextEventCloseEvent = true
      socket.terminate()
      this.setWebSocket("event", null)
      this.setWebSocketConnectPromise("event", null)
    }
    this.setRelayDaemonPublicKey("event", null)
    this.scheduleReconnect(25)
  }

  onKernelEvent(handler: (event: KernelEvent) => void) {
    this.eventHandlers.add(handler)
    return () => {
      this.eventHandlers.delete(handler)
    }
  }

  async close(): Promise<void> {
    this.terminalLocalDirect?.close()
    this.clearRuntimeTransportState("kernel client closed")
    let timedOut = false
    let timeout: ReturnType<typeof setTimeout> | undefined
    await Promise.race([
      Promise.all([
        this.relayIssuer?.close(),
        this.closeWebSocket("control"),
        this.closeWebSocket("event"),
      ]).then(() => undefined),
      new Promise<void>((resolve) => {
        timeout = setTimeout(() => {
          timedOut = true
          resolve()
        }, IPC_CLIENT_CLOSE_TIMEOUT_MS)
      }),
    ])
    if (timeout) {
      clearTimeout(timeout)
    }
    if (timedOut) {
      this.destroy()
    }
  }

  destroy(): void {
    this.terminalLocalDirect?.close()
    this.relayIssuer?.destroy()
    this.clearRuntimeTransportState("kernel client destroyed")
    this.destroyWebSocket("control")
    this.destroyWebSocket("event")
  }

  private clearRuntimeTransportState(pendingMessage: string): void {
    this.relayRenewal?.stop()
    this.relayRenewal = null
    this.requestLifetime.retire(pendingMessage)
    this.activeKernelSubscription = null
    this.clearReconnectState()
    this.clearKernelEventWatchdog()
    this.clearKernelHeartbeat("control")
    this.clearKernelHeartbeat("event")
    this.controlRelayDaemonPublicKey = null
    this.eventRelayDaemonPublicKey = null
    this.rejectPending(pendingMessage)
  }

  private async sendWebSocket<TResponse>(request: unknown, lane: KernelSocketLane = "control", admittedSocket?: WebSocket): Promise<TResponse> {
    const lifetime = this.requestLifetime.capture()
    const requestId = randomUUID()
    const waitsForAuthorization = waitsForKernelAuthorization(request)
    const retryUntilMs = lane === "control" && !waitsForAuthorization
      ? Date.now() + this.controlRequestRetryDeadlineMs
      : Date.now()
    const replayAfterWrite = !runsAgainOnReplay(request)
    let retryDelayMs = KERNEL_RECONNECT_BASE_DELAY_MS
    const relayBinding: { socket: WebSocket | null; request: ReturnType<typeof normalizeRelayRequest> | null } = { socket: null, request: null }

    for (;;) {
      lifetime.throwIfAborted()
      let socket: WebSocket
      try {
        socket = admittedSocket ?? await this.ensureWebSocket(lane)
        if (admittedSocket && (this.getWebSocket(lane) !== admittedSocket || admittedSocket.readyState !== WebSocket.OPEN)) {
          throw new LocalIpcError("admit kernel control", "Kernel connection changed after capability admission; reconcile before retrying")
        }
      } catch (error) {
        if (this.relayAuthorizationFailure) throw this.relayAuthorizationFailure
        lifetime.throwIfAborted()
        if (admittedSocket || !this.shouldReplayWebSocketRequest(error, lane, retryUntilMs)) {
          throw error
        }
        this.destroyWebSocket(lane, "kernel websocket reset", "request_replay")
        retryDelayMs = await this.waitBeforeWebSocketRequestReplay(retryDelayMs, retryUntilMs, lifetime)
        continue
      }

      lifetime.throwIfAborted()
      const pending = this.pendingRequests.register<TResponse>(
        requestId,
        lane,
        waitsForAuthorization ? 0 : replayAfterWrite ? this.requestAttemptTimeoutMs(lane, retryUntilMs) : IPC_TIMEOUT_MS,
      )

      try {
        const daemonPublicKey = this.isRelayMode()
          ? this.relayDaemonPublicKeyForSocket(lane, socket)
          : null
        // MP-08/MP-11: a delayed reply from this carrier can settle the replay.
        // Keep its ephemeral response key bound to the request until the socket changes.
        if (this.isRelayMode() && relayBinding.socket !== socket) {
          relayBinding.request = normalizeRelayRequest(
            requestId, request, this.relayTarget, daemonPublicKey, this.relayIdentity,
          )
          relayBinding.socket = socket
        }
        const relayRequest = this.isRelayMode() ? relayBinding.request : null
        if (relayRequest) {
          pending.setRelayDecryptResponse((payload) => {
            if (this.getWebSocket(lane) !== socket) {
              throw new Error("relay response belongs to a stale connection")
            }
            return relayRequest.decryptResponse(payload)
          })
        }
        const payload = relayRequest
          ? relayRequest.frame
          : normalizeWebSocketRequest(requestId, request)
        socket.send(JSON.stringify(payload))
      } catch (error) {
        pending.reject(new LocalIpcError("write kernel request", error instanceof Error ? error.message : String(error), "write_failed", true))
      }

      try {
        return await pending.promise
      } catch (error) {
        if (this.relayAuthorizationFailure) throw this.relayAuthorizationFailure
        lifetime.throwIfAborted()
        if (!replayAfterWrite && error instanceof LocalIpcError
          && (error.code === "connection_closed" || error.code === "request_timeout")) {
          const lost = error.code === "request_timeout" ? "no answer in time" : "the connection closed before the answer"
          throw new LocalIpcError("handle kernel response", `${lost}; the kernel may have run the request`, "outcome_unknown")
        }
        if (admittedSocket || !this.shouldReplayWebSocketRequest(error, lane, retryUntilMs)) {
          throw error
        }
        const current = this.getWebSocket(lane)
        const responsiveTimeout = error instanceof LocalIpcError && error.code === "request_timeout"
          && current === socket && socket.readyState === WebSocket.OPEN
          && this.socketResponsiveness.isResponsive(socket, this.kernelPingIntervalMs * this.kernelMaxMissedPongs)
        // MP-10: replay the same command on a responsive carrier. A slow
        // handler is not a transport failure, and a stale attempt cannot retire
        // a newer lane. Dead/unproven carriers retain the existing recovery.
        if (current === socket && !responsiveTimeout) {
          this.destroyWebSocket(lane, "kernel websocket reset", "request_replay", {
            code: error instanceof LocalIpcError ? error.code : null,
          })
        } else if (responsiveTimeout) {
          this.reportTransportDiagnostic(lane, "request_replay", { code: "request_timeout", operation: "retry responsive socket" })
        }
        retryDelayMs = await this.waitBeforeWebSocketRequestReplay(retryDelayMs, retryUntilMs, lifetime)
      }
    }
  }

  private shouldReplayWebSocketRequest(error: unknown, lane: KernelSocketLane, retryUntilMs: number): boolean {
    return lane === "control"
      && Date.now() < retryUntilMs
      && error instanceof LocalIpcError
      && error.retryable
      && (error.code === "connection_closed" || error.code === "write_failed" || error.code === "request_timeout")
  }

  private requestAttemptTimeoutMs(lane: KernelSocketLane, retryUntilMs: number): number {
    if (lane !== "control") {
      return IPC_TIMEOUT_MS
    }
    const remainingRetryMs = retryUntilMs - Date.now()
    if (remainingRetryMs <= this.controlResponseStallMs) {
      return IPC_TIMEOUT_MS
    }
    return this.controlResponseStallMs
  }

  private async waitBeforeWebSocketRequestReplay(delayMs: number, retryUntilMs: number, lifetime: AbortSignal): Promise<number> {
    lifetime.throwIfAborted()
    const remainingMs = retryUntilMs - Date.now()
    if (remainingMs <= 0) {
      return delayMs
    }
    const waitMs = Math.min(this.reconnectDelayWithJitter(delayMs), remainingMs)
    if (waitMs > 0) {
      await waitForKernelRequestReplay(waitMs, lifetime)
    }
    return this.nextReconnectDelayMs(delayMs)
  }

  /**
   * MP-08: binds the given subscription, never whichever one is active after
   * the socket await, so overlapping subscribes cannot share an id with
   * different keys.
   */
  private async sendRelaySubscribe(
    subscription: KernelSubscriptionState,
    resumeFromEventId: number | null,
  ): Promise<void> {
    const { sessionId, attachmentId } = subscription
    const subscriptionScope = kernelSubscriptionScopeValue(subscription)
    const lane: KernelSocketLane = "event"
    const socket = await this.ensureWebSocket(lane)
    const daemonPublicKey = this.relayDaemonPublicKeyForSocket(lane, socket)
    const requestId = randomUUID()
    if (!subscription.relaySubscriptionId) {
      throw new LocalIpcError("write relay subscribe", "relay subscription state is missing")
    }
    const subscriptionId = subscription.relaySubscriptionId
    const keypair = this.relayIdentity ? null : createRelayKeypair()
    const identity = this.relayIdentity
    const clientPublicKey = this.relayIdentity?.publicKeyBase64
      ?? keypair!.publicKeyBase64
    const decryptEvent = (payload: EncryptedRelayPayload) => {
      if (this.getWebSocket(lane) !== socket) {
        throw new Error("relay event belongs to a stale connection")
      }
      return identity
        ? identity.decrypt(payload, daemonPublicKey)
        : decryptRelayPayloadFromExpectedSender(keypair!.privateKey, payload, daemonPublicKey)
    }
    subscription.relayPublicKey = clientPublicKey
    subscription.relayDecryptEvent = decryptEvent

    const pending = this.pendingRequests.register<void>(requestId, lane)
    pending.setRelayDecryptResponse(decryptEvent)

    try {
      const frame = buildRelaySubscribeFrame({
        requestId,
        subscriptionId,
        target: this.relayTarget,
        sessionId,
        attachmentId,
        clientPublicKey,
        resumeFromEventId,
        subscriptionScope,
      })
      socket.send(JSON.stringify(frame))
    } catch (error) {
      pending.reject(new LocalIpcError("write relay subscribe", error instanceof Error ? error.message : String(error), "write_failed", true))
    }

    await pending.promise
  }

  private async sendRelayUnsubscribe(
    subscriptionId: string,
    clientPublicKey: string,
    decryptResponse: (payload: EncryptedRelayPayload) => string,
  ): Promise<void> {
    const lane: KernelSocketLane = "event"
    const socket = await this.ensureWebSocket(lane)
    const requestId = randomUUID()

    const pending = this.pendingRequests.register<void>(requestId, lane)
    pending.setRelayDecryptResponse(decryptResponse)

    try {
      const frame = buildRelayUnsubscribeFrame(requestId, subscriptionId, clientPublicKey)
      socket.send(JSON.stringify(frame))
    } catch (error) {
      pending.reject(new LocalIpcError("write relay unsubscribe", error instanceof Error ? error.message : String(error), "write_failed", true))
    }

    await pending.promise
  }

  private openKernelWebSocket(): WebSocket {
    if (this.socketPath.startsWith("ws+unix://")) {
      const socket = this.socketPath.slice("ws+unix://".length)
      if (!socket.startsWith("/") || socket.includes(":") || socket.includes("?")) {
        throw new Error("ws+unix endpoint must name an absolute Unix socket path")
      }
      return new WebSocket(`ws+unix:${socket}:/kernel`)
    }
    if (this.isRelayMode()) {
      return new WebSocket(this.socketPath)
    }
    if (this.localAuthToken && this.localAuthEndpoint) {
      return new WebSocket(this.localAuthEndpoint, {
        headers: { authorization: `Bearer ${this.localAuthToken}` },
      })
    }
    // A laptop kernel writes a new token at each start, so read it for every
    // connection: a reconnect after a kernel restart presents the new one.
    // A missing or stale token receives a non-retryable authentication error.
    const laptopKernelToken = readLocalKernelAuthToken(this.socketPath, this.localAuthEnvironment)
    return laptopKernelToken
      ? new WebSocket(this.socketPath, { headers: { authorization: `Bearer ${laptopKernelToken}` } })
      : new WebSocket(this.socketPath)
  }

  private async ensureWebSocket(lane: KernelSocketLane = "control"): Promise<WebSocket> {
    if (this.relayAuthorizationFailure) throw this.relayAuthorizationFailure
    const existing = this.getWebSocket(lane)
    if (existing?.readyState === WebSocket.OPEN) {
      return existing
    }
    const connectPromise = this.getWebSocketConnectPromise(lane)
    if (connectPromise) {
      return connectPromise
    }

    const nextConnectPromise = new Promise<WebSocket>((resolve, reject) => {
      let socket = this.openKernelWebSocket()
      let settled = false
      this.setConnectingWebSocket(lane, socket)

      const fail = (operation: string, error: unknown, code: string | null = null, retryable = false) => {
        if (settled) {
          return
        }
        settled = true
        if (this.getConnectingWebSocket(lane) === socket) {
          this.setConnectingWebSocket(lane, null)
        }
        this.setWebSocketConnectPromise(lane, null)
        reject(new LocalIpcError(operation, formatTransportError(error, this.socketPath), code, retryable))
        if (socket.readyState !== WebSocket.CLOSED) {
          socket.terminate()
        }
      }

      const handleConnectError = (error: unknown) => {
        const authenticationFailed = /Unexpected server response: (?:401|403)/i.test(String(error))
        fail(
          "connect kernel websocket",
          error,
          authenticationFailed ? "authentication_failed" : "connection_closed",
          !authenticationFailed,
        )
      }
      const handleUnexpectedResponse = kernelUpgradeRejectionHandler(socket, (message, authenticationFailed) => {
        fail("connect kernel websocket", message, authenticationFailed ? "authentication_failed" : "connection_closed", !authenticationFailed)
      })
      const handleConnectClose = (code: number, reason: Buffer) => {
        const closeMessage = reason.length > 0
          ? reason.toString("utf8")
          : `kernel websocket closed before opening${code ? ` (${code})` : ""}`
        fail("connect kernel websocket", closeMessage, "connection_closed", true)
      }
      const clearConnectListeners = () => {
        socket.off("error", handleConnectError)
        socket.off("close", handleConnectClose)
        socket.off("unexpected-response", handleUnexpectedResponse)
      }

      socket.once("open", () => {
        const finalizeOpen = () => {
          if (settled || this.getConnectingWebSocket(lane) !== socket) {
            return
          }
          settled = true
          clearConnectListeners()
          if (this.getConnectingWebSocket(lane) === socket) {
            this.setConnectingWebSocket(lane, null)
          }
          this.setWebSocket(lane, socket)
          this.setWebSocketConnectPromise(lane, null)
          this.setSuppressNextCloseEvent(lane, false)
          this.startKernelHeartbeat(socket, lane)
          socket.on("message", (data: WebSocket.RawData) => {
            if (this.getWebSocket(lane) !== socket) {
              return
            }
            this.handleWebSocketMessage(data, lane)
          })
          socket.on("pong", () => {
            if (this.getWebSocket(lane) !== socket) return
            this.socketResponsiveness.recordPong(socket)
            this.setMissedKernelPongs(lane, 0)
          })
          socket.once("close", (code: number, reason: Buffer) => {
            if (this.getWebSocket(lane) !== socket) {
              return
            }
            this.reportTransportDiagnostic(lane, "socket_close", { closeCode: code })
            const suppressed = this.getSuppressNextCloseEvent(lane)
            this.setSuppressNextCloseEvent(lane, false)
            const closeMessage = reason.length > 0
              ? reason.toString("utf8")
              : `kernel websocket closed${code ? ` (${code})` : ""}`
            if (this.relayRenewal && /relay token (?:has been )?(?:expired|revoked)/i.test(closeMessage)) {
              this.refuseRelayAuthorization()
              return
            }
            this.rejectPending(closeMessage, lane)
            this.setWebSocket(lane, null)
            this.setRelayDaemonPublicKey(lane, null)
            this.clearKernelHeartbeat(lane)
            if (!suppressed) {
              this.emitSyntheticEvent({
                event: "transport_closed",
                message: closeMessage,
              })
              if (lane === "event") {
                this.scheduleReconnect()
              }
            }
          })
          socket.on("error", (error: unknown) => {
            if (this.getWebSocket(lane) !== socket) {
              return
            }
            this.reportTransportDiagnostic(lane, "socket_error")
            const message = formatTransportError(error, this.socketPath)
            const suppressed = this.getSuppressNextCloseEvent(lane)
            this.setSuppressNextCloseEvent(lane, false)
            this.rejectPending(message, lane)
            this.setWebSocket(lane, null)
            this.setRelayDaemonPublicKey(lane, null)
            this.clearKernelHeartbeat(lane)
            if (!suppressed) {
              this.emitSyntheticEvent({
                event: "transport_closed",
                message,
              })
              if (lane === "event") {
                this.scheduleReconnect()
              }
            }
          })
          resolve(socket)
          this.startRelayAuthorizationRenewal()
        }

        if (!this.isRelayMode()) {
          finalizeOpen()
          return
        }

        const handleRelayHandshakeMessage = (data: WebSocket.RawData) => {
          if (this.getConnectingWebSocket(lane) !== socket) {
            return
          }
          let frame: RelayConnectedFrame | RelayCloseFrame
          try {
            frame = JSON.parse(String(data)) as RelayConnectedFrame | RelayCloseFrame
          } catch (error) {
            fail("connect relay transport", error)
            return
          }
          if (!frame || typeof frame !== "object" || Array.isArray(frame)) {
            fail("connect relay transport", "unexpected relay handshake frame")
            return
          }
          if (frame.kind === "client_connected") {
            if (!relayTargetMatches(frame.target, this.relayTarget)) {
              fail("connect relay transport", "relay connected to a different daemon target")
              return
            }
            if (
              typeof frame.daemon_public_key !== "string"
              || frame.daemon_public_key.trim() === ""
              || frame.daemon_public_key.trim() !== frame.daemon_public_key
            ) {
              fail("connect relay transport", "relay did not provide daemon public key")
              return
            }
            this.setRelayDaemonPublicKey(lane, frame.daemon_public_key)
            socket.off("message", handleRelayHandshakeMessage)
            const relaySocket = socket
            void (async () => {
              const direct = await this.terminalLocalDirect?.open(frame.daemon_public_key) ?? null
              if (settled || this.getConnectingWebSocket(lane) !== relaySocket) {
                direct?.terminate()
                return
              }
              if (direct && direct.readyState === WebSocket.OPEN) {
                clearConnectListeners()
                socket = direct
                this.setConnectingWebSocket(lane, direct)
                relaySocket.terminate()
              }
              finalizeOpen()
            })().catch(error => fail("connect terminal carrier", error))
            return
          }
          if (frame.kind === "close") {
            const error = relayCloseError(frame.reason)
            fail("connect relay transport", error.message, error.code, error.retryable)
            return
          }
          fail("connect relay transport", "unexpected relay handshake frame")
        }

        socket.on("message", handleRelayHandshakeMessage)
        try {
          socket.send(JSON.stringify(buildRelayConnectFrame(this.relayAuthToken, this.relayTarget)))
        } catch (error) {
          socket.off("message", handleRelayHandshakeMessage)
          fail("write relay connect frame", error, "write_failed", true)
        }
      })

      socket.on("unexpected-response", handleUnexpectedResponse)
      socket.on("error", handleConnectError)
      socket.on("close", handleConnectClose)
    })

    this.setWebSocketConnectPromise(lane, nextConnectPromise)
    return nextConnectPromise
  }

  private startRelayAuthorizationRenewal(): void {
    if (this.relayRenewal || this.relayAuthorizationFailure || !this.relayIdentity) return
    const claims = relayAuthorization(this.relayAuthToken)
    if (!claims?.account_id || !claims.user_id) return // Local/operator transports have no Cloud lifetime.
    if (claims.public_key_thumbprint !== this.relayIdentity.publicKeyThumbprint) return
    const renewal: RelayAuthorizationRenewal = new RelayAuthorizationRenewal(claims.exp * 1000,
      (): Promise<number> => this.renewRelayAuthorization(renewal).catch(error => {
        this.reportTransportDiagnostic("control", "renewal_failed", error instanceof LocalIpcError
          ? { operation: error.operation, code: error.code, retryable: error.retryable } : {})
        throw error
      }), message => this.refuseRelayAuthorization(message), message => {
        this.relayRenewalNotice(message)
      })
    this.relayRenewal = renewal
    // Probe immediately, before the user relies on silent renewal. Capability
    // negotiation also handles kernels on branches with unrelated version bumps.
    void this.requireRelayRenewalSupport().catch(error => {
      if (this.relayRenewal === renewal && error instanceof LocalIpcError && error.code === "relay_renewal_issuer_unavailable") this.relayRenewalNotice(error.message)
      if (this.relayRenewal === renewal && error instanceof LocalIpcError && !error.retryable) this.refuseRelayAuthorization(error.code === "kernel_upgrade_required" ? error.message : undefined)
    })
  }

  private async requireRelayRenewalSupport(): Promise<void> {
    if (this.relayRenewalNegotiated && !this.relayIssuer) return
    const response = await this.sendToRelayIssuer<{RelayStatus: {status: {capabilities?: string[]; daemon_id?: string}}}>({RelayStatus: null})
    if (this.relayIssuer && response.RelayStatus?.status?.daemon_id !== this.relayIssuerDaemonId) throw new LocalIpcError("negotiate relay renewal", "The original relay issuing kernel identity changed", "authorization_denied", false)
    if (!response.RelayStatus?.status?.capabilities?.includes(relayAuthorizationRenewalCapability)) {
      throw new LocalIpcError("negotiate relay renewal", `Hosted terminal renewal requires kernel protocol ${relayAuthorizationRenewalMinimumProtocolVersion} or newer; update the kernel before attaching this terminal.`, "kernel_upgrade_required", false)
    }
    this.relayRenewalNegotiated = true
  }

  private async renewRelayAuthorization(renewal: RelayAuthorizationRenewal): Promise<number> {
    await this.requireRelayRenewalSupport()
    const previous = relayAuthorization(this.relayAuthToken)!
    const target = this.relayTarget?.daemon_id ?? this.relayTarget?.daemon_alias
    if (!target || !this.relayIdentity) throw new LocalIpcError("renew relay authorization", "Relay authorization is invalid", "authorization_denied")
    // The kernel keeps its Cloud authority private and performs the existing
    // control-plane issuance call. Runtime traffic remains encrypted via relay.
    const reply = await this.sendToRelayIssuer<{CloudRelayClientTokenIssued: {token: {relay_url: string; relay_token: string}}}>(
      issueCloudRelayClientTokenRequest(target, previous.sub, previous.session_id, this.relayIdentity.publicKeyThumbprint))
    const grant = reply.CloudRelayClientTokenIssued?.token
    if (!grant || grant.relay_url !== this.socketPath) throw new LocalIpcError("renew relay authorization", "Relay authorization renewal targets an invalid relay", "authorization_denied")
    const next = requireRenewedRelayAuthorization(previous, grant.relay_token, target)
    if (this.relayAuthorizationFailure || this.relayRenewal !== renewal) throw new LocalIpcError("renew relay authorization", "Renewal was cancelled", "client_closed")
    this.relayAuthToken = grant.relay_token
    this.relayIssuerNoticeSent = false
    // A lane opening concurrently must finish its handshake, then receive the
    // same grant as the retained lane. Future reconnects use the new token.
    await Promise.all((["control", "event"] as const).map(async lane => {
      const connecting = this.getWebSocketConnectPromise(lane)
      if (connecting) await connecting
      const socket = this.getWebSocket(lane)
      // A direct carrier is authorized by its relay-renewed lease, not this grant.
      if (socket?.readyState === WebSocket.OPEN && !this.terminalLocalDirect?.isLocal(socket)) await reauthenticateRelaySocket(socket, this.relayAuthToken!, this.relayTarget!, this.relayDaemonPublicKeyForSocket(lane, socket), this.requestLifetime.capture())
    }))
    return next.exp * 1000
  }

  private relayRenewalNotice(message: string): void {
    if (this.relayIssuerNoticeSent) return
    this.relayIssuerNoticeSent = true
    this.emitSyntheticEvent({event: "runtime_notices", notices: [{message}]})
  }

  private async sendToRelayIssuer<T>(request: unknown): Promise<T> {
    try { return await (this.relayIssuer ?? this).send<T>(request) }
    catch (error) {
      if (this.relayIssuer && error instanceof LocalIpcError && (error.retryable || error.code === "outcome_unknown" || error.code === "authentication_failed")) {
        const expires = new Date(relayAuthorization(this.relayAuthToken)!.exp * 1000).toISOString()
        throw new LocalIpcError("renew relay authorization", `The original issuing kernel is unavailable. The current connection remains valid until ${expires}; renewal will retry without changing its authority.`, "relay_renewal_issuer_unavailable", true)
      }
      throw error
    }
  }

  private refuseRelayAuthorization(upgradeMessage?: string): void {
    if (this.relayAuthorizationFailure) return
    this.reportTransportDiagnostic("control", "authorization_ended")
    const message = upgradeMessage ?? "Relay authorization renewal was refused or access was revoked. Connection ended; sign in or pair again."
    this.relayAuthorizationFailure = new LocalIpcError("renew relay authorization", message, "authorization_denied", false)
    this.destroy()
    this.emitSyntheticEvent({event: "transport_closed", message})
  }

  private handleWebSocketMessage(data: WebSocket.RawData, lane: KernelSocketLane) {
    let frame:
      | KernelTransportResponseFrame<unknown>
      | KernelTransportEventFrame<KernelEvent>
      | RelayResponseFrame<unknown>
      | RelayEventFrame
      | RelayCloseFrame
      | RelayConnectedFrame
    try {
      frame = JSON.parse(String(data)) as
        | KernelTransportResponseFrame<unknown>
        | KernelTransportEventFrame<KernelEvent>
        | RelayResponseFrame<unknown>
        | RelayEventFrame
        | RelayCloseFrame
        | RelayConnectedFrame
    } catch (error) {
      this.rejectPending(error instanceof Error ? error.message : String(error), lane)
      return
    }

    if ("type" in frame && frame.type === "event") {
      let event: KernelEvent
      try {
        event = kernelEventFromValue(frame.event)
      } catch (error) {
        this.rejectPending(error instanceof Error ? error.message : String(error), lane)
        return
      }
      this.lastReceivedEventId = frame.event_id
      this.markKernelEventReceived()
      for (const handler of this.eventHandlers) {
        handler(event)
      }
      return
    }

    if ("kind" in frame && frame.kind === "client_connected") return // The renewal waiter validates this acknowledgement.

    if ("kind" in frame && frame.kind === "close") {
      if (this.relayRenewal && /relay token (?:has been )?(?:expired|revoked)/i.test(frame.reason)) {
        this.refuseRelayAuthorization()
        return
      }
      this.rejectPending(frame.reason, lane)
      return
    }

    if ("kind" in frame && frame.kind === "client_event") {
      const subscription = this.activeKernelSubscription
      if (!subscription?.relayDecryptEvent || subscription.relaySubscriptionId !== frame.subscription_id) {
        return
      }
      let decrypted: string
      try {
        decrypted = subscription.relayDecryptEvent(frame.encrypted_event)
      } catch {
        // MP-08: an event sealed for a superseded binding of this subscription
        // (or a stale connection) is dropped; it must not fail the lane.
        return
      }
      try {
        const event = kernelEventFromValue(JSON.parse(decrypted))
        this.lastReceivedEventId = frame.event_id
        this.markKernelEventReceived()
        this.emitSyntheticEvent(event)
      } catch (error) {
        this.rejectPending(error instanceof Error ? error.message : String(error), lane)
      }
      return
    }

    const requestId = "type" in frame ? frame.request_id : frame.request_id
    const pending = this.pendingRequests.take(requestId)
    if (!pending) {
      return
    }

    if (frame.error) {
      pending.reject(new LocalIpcError("handle kernel response", frame.error.message, frame.error.code, frame.error.retryable))
      return
    }
    if ("kind" in frame) {
      if (!pending.relayDecryptResponse) {
        pending.reject(new LocalIpcError("handle kernel response", "missing relay request key"))
        return
      }
      if (frame.encrypted_response == null) {
        pending.reject(new LocalIpcError("handle kernel response", "response envelope was empty"))
        return
      }
      try {
        const decrypted = pending.relayDecryptResponse(frame.encrypted_response)
        pending.resolve(JSON.parse(decrypted) as unknown)
      } catch (error) {
        pending.reject(new LocalIpcError("handle kernel response", error instanceof Error ? error.message : String(error)))
      }
      return
    }
    if (frame.response == null) {
      pending.reject(new LocalIpcError("handle kernel response", "response envelope was empty"))
      return
    }

    pending.resolve(frame.response)
  }

  private rejectPending(message: string, lane?: KernelSocketLane) {
    this.pendingRequests.rejectMatching(message, lane)
  }

  private emitSyntheticEvent(event: KernelEvent) {
    for (const handler of this.eventHandlers) {
      handler(event)
    }
  }

  private clearReconnectState() {
    if (this.reconnectTimeout) {
      clearTimeout(this.reconnectTimeout)
      this.reconnectTimeout = null
    }
    this.reconnectDelayMs = KERNEL_RECONNECT_BASE_DELAY_MS
  }

  private markKernelEventReceived() {
    this.lastKernelEventAtMs = Date.now()
    this.armKernelEventWatchdog()
  }

  private armKernelEventWatchdog() {
    this.clearKernelEventWatchdog()
    if (!this.kernelEventStaleMs || !this.activeKernelSubscription || this.eventHandlers.size === 0) {
      return
    }
    this.kernelEventWatchdog = setTimeout(() => {
      const elapsedMs = Date.now() - this.lastKernelEventAtMs
      if (!this.activeKernelSubscription || this.eventHandlers.size === 0) {
        return
      }
      if (elapsedMs < this.kernelEventStaleMs) {
        this.armKernelEventWatchdog()
        return
      }
      this.emitSyntheticEvent({
        event: "transport_closed",
        message: `kernel event stream stalled for ${elapsedMs}ms; reconnecting`,
      })
      void this.restartKernelEventStream()
    }, this.kernelEventStaleMs)
  }

  private clearKernelEventWatchdog() {
    if (this.kernelEventWatchdog) {
      clearTimeout(this.kernelEventWatchdog)
      this.kernelEventWatchdog = null
    }
  }

  private startKernelHeartbeat(socket: WebSocket, lane: KernelSocketLane) {
    this.clearKernelHeartbeat(lane)
    this.setMissedKernelPongs(lane, 0)
    const heartbeat = setInterval(() => {
      if (socket !== this.getWebSocket(lane) || socket.readyState !== WebSocket.OPEN) {
        clearInterval(heartbeat)
        return
      }
      if (this.getMissedKernelPongs(lane) >= this.kernelMaxMissedPongs) {
        if (lane === "event") {
          this.emitSyntheticEvent({
            event: "transport_closed",
            message: "kernel websocket heartbeat missed; reconnecting",
          })
        }
        this.destroyWebSocket(lane, "kernel websocket heartbeat missed", "heartbeat_missed")
        if (lane === "event") {
          this.scheduleReconnect()
        }
        return
      }
      this.setMissedKernelPongs(lane, this.getMissedKernelPongs(lane) + 1)
      try {
        socket.ping()
      } catch {
        if (lane === "event") {
          this.emitSyntheticEvent({
            event: "transport_closed",
            message: "kernel websocket heartbeat failed; reconnecting",
          })
        }
        this.destroyWebSocket(lane, "kernel websocket heartbeat failed", "heartbeat_failed")
        if (lane === "event") {
          this.scheduleReconnect()
        }
      }
    }, this.kernelPingIntervalMs)
    if (lane === "control") {
      this.controlHeartbeat = heartbeat
    } else {
      this.eventHeartbeat = heartbeat
    }
  }

  private clearKernelHeartbeat(lane: KernelSocketLane) {
    const heartbeat = lane === "control" ? this.controlHeartbeat : this.eventHeartbeat
    if (heartbeat) {
      clearInterval(heartbeat)
      if (lane === "control") {
        this.controlHeartbeat = null
      } else {
        this.eventHeartbeat = null
      }
    }
    this.setMissedKernelPongs(lane, 0)
  }

  private scheduleReconnect(delayMs = this.reconnectDelayMs) {
    if (!this.activeKernelSubscription || this.eventHandlers.size === 0 || this.reconnectTimeout) {
      return
    }

    this.reconnectTimeout = setTimeout(() => {
      this.reconnectTimeout = null
      void this.resumeKernelSubscription()
    }, this.reconnectDelayWithJitter(delayMs))
    this.reconnectDelayMs = this.nextReconnectDelayMs(delayMs)
  }

  private reconnectDelayWithJitter(delayMs: number): number {
    const boundedDelayMs = Math.max(delayMs, 0)
    if (boundedDelayMs < KERNEL_RECONNECT_BASE_DELAY_MS || this.reconnectJitterMs === 0) {
      return boundedDelayMs
    }
    const jitterMs = Math.floor(clampRandom(this.reconnectRandom()) * this.reconnectJitterMs)
    return Math.min(boundedDelayMs + jitterMs, KERNEL_RECONNECT_MAX_DELAY_MS + this.reconnectJitterMs)
  }

  private nextReconnectDelayMs(delayMs: number): number {
    return Math.min(
      Math.max(delayMs * 2, KERNEL_RECONNECT_BASE_DELAY_MS),
      KERNEL_RECONNECT_MAX_DELAY_MS,
    )
  }

  private async resumeKernelSubscription() {
    const subscription = this.activeKernelSubscription
    if (!subscription || this.eventHandlers.size === 0) {
      return
    }

    try {
      if (this.isRelayMode()) {
        await this.sendRelaySubscribe(subscription, this.lastReceivedEventId)
      } else {
        await this.sendWebSocket<Record<string, unknown>>(
          buildKernelSubscriptionTransportRequest(subscription, this.lastReceivedEventId),
          "event",
        )
      }
      this.clearReconnectState()
      this.markKernelEventReceived()
      this.emitSyntheticEvent({
        event: "transport_resumed",
        session_id: subscription.sessionId,
        resumed_from_event_id: this.lastReceivedEventId,
      })
    } catch {
      this.scheduleReconnect()
    }
  }

  private getWebSocket(lane: KernelSocketLane) {
    return lane === "control" ? this.controlWebsocket : this.eventWebsocket
  }

  /** Connection-local state can reset without becoming an event consumer. */
  protected onControlConnectionChanged(): void {}

  private setWebSocket(lane: KernelSocketLane, socket: WebSocket | null) {
    if (lane === "control") {
      const changed = this.controlWebsocket !== socket
      this.controlWebsocket = socket
      if (changed) {
        this.relayRenewalNegotiated = false
        this.onControlConnectionChanged()
      }
    } else {
      this.eventWebsocket = socket
    }
  }

  private getConnectingWebSocket(lane: KernelSocketLane) {
    return lane === "control" ? this.connectingControlWebsocket : this.connectingEventWebsocket
  }

  private setConnectingWebSocket(lane: KernelSocketLane, socket: WebSocket | null) {
    if (lane === "control") {
      this.connectingControlWebsocket = socket
    } else {
      this.connectingEventWebsocket = socket
    }
  }

  private getWebSocketConnectPromise(lane: KernelSocketLane) {
    return lane === "control" ? this.controlWebsocketConnectPromise : this.eventWebsocketConnectPromise
  }

  private setWebSocketConnectPromise(lane: KernelSocketLane, promise: Promise<WebSocket> | null) {
    if (lane === "control") {
      this.controlWebsocketConnectPromise = promise
    } else {
      this.eventWebsocketConnectPromise = promise
    }
  }

  private getRelayDaemonPublicKey(lane: KernelSocketLane) {
    return lane === "control" ? this.controlRelayDaemonPublicKey : this.eventRelayDaemonPublicKey
  }

  private relayDaemonPublicKeyForSocket(lane: KernelSocketLane, socket: WebSocket): string {
    if (this.getWebSocket(lane) !== socket) {
      throw new LocalIpcError(
        "encrypt relay request",
        "relay connection changed before the request was sent",
        "connection_closed",
        true,
      )
    }
    const publicKey = this.getRelayDaemonPublicKey(lane)
    if (!publicKey) {
      throw new LocalIpcError(
        "encrypt relay request",
        "relay daemon public key is missing for the active connection",
      )
    }
    return publicKey
  }

  private setRelayDaemonPublicKey(lane: KernelSocketLane, publicKey: string | null) {
    if (lane === "control") {
      this.controlRelayDaemonPublicKey = publicKey
    } else {
      this.eventRelayDaemonPublicKey = publicKey
    }
  }

  private getSuppressNextCloseEvent(lane: KernelSocketLane) {
    return lane === "control" ? this.suppressNextControlCloseEvent : this.suppressNextEventCloseEvent
  }

  private setSuppressNextCloseEvent(lane: KernelSocketLane, value: boolean) {
    if (lane === "control") {
      this.suppressNextControlCloseEvent = value
    } else {
      this.suppressNextEventCloseEvent = value
    }
  }

  private getMissedKernelPongs(lane: KernelSocketLane) {
    return lane === "control" ? this.missedControlPongs : this.missedEventPongs
  }

  private setMissedKernelPongs(lane: KernelSocketLane, value: number) {
    if (lane === "control") {
      this.missedControlPongs = value
    } else {
      this.missedEventPongs = value
    }
  }

  private async closeWebSocket(lane: KernelSocketLane): Promise<void> {
    const socket = this.getWebSocket(lane) ?? this.getConnectingWebSocket(lane)
    this.setWebSocket(lane, null)
    this.setConnectingWebSocket(lane, null)
    this.setWebSocketConnectPromise(lane, null)
    this.setRelayDaemonPublicKey(lane, null)
    if (!socket || socket.readyState === WebSocket.CLOSED) {
      return
    }

    await new Promise<void>((resolve) => {
      let settled = false
      let timeout: ReturnType<typeof setTimeout> | undefined
      const finish = () => {
        if (settled) {
          return
        }
        settled = true
        if (timeout) {
          clearTimeout(timeout)
        }
        resolve()
      }
      timeout = setTimeout(() => {
        socket.terminate()
        finish()
      }, IPC_WEBSOCKET_CLOSE_TIMEOUT_MS)
      this.setSuppressNextCloseEvent(lane, true)
      socket.once("close", finish)
      if (socket.readyState === WebSocket.CONNECTING) {
        socket.terminate()
      } else {
        socket.close()
      }
    })
  }

  private reportTransportDiagnostic(lane: KernelSocketLane, cause: KernelTransportDiagnostic["cause"],
    details: Partial<KernelTransportDiagnostic> = {}): void {
    try {
      this.transportObserver?.({ lane, cause, local: this.terminalLocalDirect?.isLocal(this.getWebSocket(lane)) === true,
        missedPongs: this.getMissedKernelPongs(lane), ...details })
    } catch { /* Observers cannot change transport behavior. */ }
  }

  private destroyWebSocket(lane: KernelSocketLane, message = "kernel websocket reset",
    cause: KernelTransportDiagnostic["cause"] = "reset", details: Partial<KernelTransportDiagnostic> = {}): void {
    if (this.getWebSocket(lane)) this.reportTransportDiagnostic(lane, cause, details)
    // Retiring the lane makes its asynchronous close/error callbacks stale.
    // Settle requests here, including untimed human authorization waits.
    this.rejectPending(message, lane)
    const socket = this.getWebSocket(lane) ?? this.getConnectingWebSocket(lane)
    this.setWebSocket(lane, null)
    this.setConnectingWebSocket(lane, null)
    this.setWebSocketConnectPromise(lane, null)
    this.setRelayDaemonPublicKey(lane, null)
    this.clearKernelHeartbeat(lane)
    if (socket && socket.readyState !== WebSocket.CLOSED) {
      this.setSuppressNextCloseEvent(lane, true)
      socket.terminate()
    }
    // The dropped socket's close handler no longer sees it as the lane's, so
    // its other requests end here: a replayable one is resent now, and one the
    // kernel runs again (a worker control) is not left waiting for 600 s.
    this.rejectPending("kernel websocket dropped", lane)
  }
}

function runsAgainOnReplay(request: unknown): boolean {
  return request !== null && typeof request === "object"
    && Object.keys(request).some((kind) => KERNEL_REQUESTS_RUN_AGAIN_ON_REPLAY.has(kind))
}

function kernelEventFromValue(value: unknown): KernelEvent {
  const eventName = value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>).event
    : null
  if (typeof eventName !== "string" || !eventName.trim()) {
    throw new Error("kernel event envelope must contain a non-empty event name")
  }
  return value as KernelEvent
}

function relayTargetMatches(actual: unknown, expected: RelayTarget | null): boolean {
  if (!expected || !actual || typeof actual !== "object" || Array.isArray(actual)) {
    return false
  }
  const target = actual as Record<string, unknown>
  return (target.daemon_id ?? null) === (expected.daemon_id ?? null)
    && (target.daemon_alias ?? null) === (expected.daemon_alias ?? null)
}

function clampRandom(value: number): number {
  if (!Number.isFinite(value)) {
    return 0
  }
  return Math.min(Math.max(value, 0), 0.999999)
}
