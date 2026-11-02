import {
  createHash,
  createPrivateKey,
  createPublicKey,
  randomBytes,
  sign as cryptoSign,
} from "node:crypto";

const PRIVATE_KEY_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const MAX_SIGNATURE_LIFETIME_SECONDS = 300;
const DIRECTORY_PATH = "/.well-known/http-message-signatures-directory";

function badConfig() {
  return new Error("invalid Web Bot Auth signer configuration");
}

function isLoopbackHost(hostname) {
  const host = hostname.toLowerCase().replace(/^\[|\]$/g, "");
  if (host === "localhost" || host.endsWith(".localhost") || host === "::1") return true;
  const octets = host.split(".");
  return octets.length === 4 && octets.every((part) => /^\d{1,3}$/.test(part) && Number(part) <= 255)
    && Number(octets[0]) === 127;
}

function originValue(value) {
  if (typeof value !== "string") throw badConfig();
  let parsed;
  try { parsed = new URL(value); } catch { throw badConfig(); }
  if (!(parsed.origin === value && !parsed.username && !parsed.password
      && (parsed.protocol === "https:" || (parsed.protocol === "http:" && isLoopbackHost(parsed.hostname))))) throw badConfig();
  return value;
}

export function createWebBotAuthSigner(input) {
  if (!input || typeof input !== "object" || Array.isArray(input)) throw badConfig();
  const encodedSeed = input.private_key;
  const signatureAgent = originValue(input.signature_agent);
  const notBefore = input.not_before;
  const expiresAt = input.expires_at;
  const projectId = input.project_id;
  if (typeof encodedSeed !== "string" || !/^[A-Za-z0-9_-]{43}$/.test(encodedSeed)
      || !Number.isSafeInteger(notBefore) || !Number.isSafeInteger(expiresAt)
      || notBefore < 0 || expiresAt <= notBefore || expiresAt - notBefore > 86_400
      || (projectId !== undefined && (typeof projectId !== "string" || !projectId))) throw badConfig();
  const seed = Buffer.from(encodedSeed, "base64url");
  if (seed.length !== 32 || seed.toString("base64url") !== encodedSeed) {
    seed.fill(0);
    throw badConfig();
  }
  // The command object is transient input. Drop its encoded copy immediately;
  // the owned byte buffer is zeroed when this signer is retired.
  input.private_key = "";
  const privateKey = createPrivateKey({
    key: Buffer.concat([PRIVATE_KEY_PREFIX, seed]),
    format: "der",
    type: "pkcs8",
  });
  const jwk = createPublicKey(privateKey).export({ format: "jwk" });
  const canonicalJwk = JSON.stringify({ crv: jwk.crv, kty: jwk.kty, x: jwk.x });
  const keyId = createHash("sha256").update(canonicalJwk).digest("base64url");
  return {
    seed,
    keyId,
    signatureAgent,
    notBefore,
    expiresAt,
    projectId,
    policyRevision: input.policy_revision,
    active: true,
    destroy() {
      this.active = false;
      this.seed.fill(0);
    },
  };
}

function sfString(value) {
  return `"${value.replaceAll("\\", "\\\\").replaceAll('"', '\\"')}"`;
}

export function signWebBotAuthRequest(request, signer, nowSeconds = Math.floor(Date.now() / 1000)) {
  let url;
  try { url = new URL(request.url); } catch { throw new Error("invalid HTTP request URL"); }
  if (!["http:", "https:"].includes(url.protocol)) return null;
  if (url.username || url.password || (url.protocol === "http:" && !isLoopbackHost(url.hostname))) {
    throw new Error("signed requests require HTTPS (HTTP is allowed only on loopback) and cannot contain credentials");
  }
  if (!signer?.active || !Number.isSafeInteger(nowSeconds)
      || nowSeconds < signer.notBefore || nowSeconds >= signer.expiresAt) {
    throw new Error("Web Bot Auth signing key is not currently valid");
  }
  if (typeof request.method !== "string" || !request.method) throw new Error("invalid HTTP request method");
  url.hash = "";

  const created = nowSeconds;
  const expires = Math.min(created + MAX_SIGNATURE_LIFETIME_SECONDS, signer.expiresAt);
  if (expires <= created) throw new Error("Web Bot Auth signing key expires before a request can be signed");
  const nonce = randomBytes(24).toString("base64url");
  const components = '("@authority" "@method" "@target-uri" "signature-agent";key="sig1")';
  const params = `${components};created=${created};expires=${expires};nonce=${sfString(nonce)};keyid=${sfString(signer.keyId)};alg="ed25519";tag="web-bot-auth"`;
  const signatureBase = [
    `"@authority": ${url.host}`,
    `"@method": ${request.method}`,
    `"@target-uri": ${url.href}`,
    `"signature-agent";key="sig1": ${sfString(`${signer.signatureAgent}${DIRECTORY_PATH}`)}`,
    `"@signature-params": ${params}`,
  ].join("\n");
  const privateKey = createPrivateKey({
    key: Buffer.concat([PRIVATE_KEY_PREFIX, signer.seed]),
    format: "der",
    type: "pkcs8",
  });
  const signature = cryptoSign(null, Buffer.from(signatureBase), privateKey).toString("base64");
  return {
    "Signature-Agent": `sig1=${sfString(`${signer.signatureAgent}${DIRECTORY_PATH}`)}`,
    "Signature-Input": `sig1=${params}`,
    Signature: `sig1=:${signature}:`,
  };
}

function cdpHeaders(request, signedHeaders) {
  const headers = request?.headers;
  const entries = headers && typeof headers === "object" ? Object.entries(headers) : [];
  const replaced = new Set(["signature-agent", "signature-input", "signature"]);
  return [
    ...entries.filter(([name]) => !replaced.has(name.toLowerCase())).map(([name, value]) => ({ name, value: String(value) })),
    ...Object.entries(signedHeaders ?? {}).map(([name, value]) => ({ name, value })),
  ];
}

// Owns Fetch pauses for configured page sessions. Event dispatch stays
// synchronous; each pause gets an independent task so CDP replies can be read.
export class BrowserWebBotAuth {
  constructor(connectionProvider) {
    this.connectionProvider = connectionProvider;
    this.byTab = new Map();
    this.bySession = new Map();
    this.pendingBySession = new Map();
    this.sessionByTab = new Map();
    this.unsignedSessions = new Set();
    this.policyRevision = 0;
    this.allowedProjects = null;
  }

  async configureTab(tabId, input) {
    if (typeof tabId !== "string" || !tabId) throw badConfig();
    const previous = this.byTab.get(tabId);
    if (input === null) {
      if (!previous) return { configured: true };
      previous.active = false;
      const sessionId = this.sessionByTab.get(tabId);
      if (sessionId) {
        this.unsignedSessions.add(sessionId);
        this.bySession.delete(sessionId);
        this.sessionByTab.delete(tabId);
        try {
          await this.connectionProvider()?.send("Fetch.disable", {}, sessionId);
          this.unsignedSessions.delete(sessionId);
        } finally {
          previous.destroy();
          this.byTab.delete(tabId);
        }
      } else {
        previous.destroy();
        this.byTab.delete(tabId);
      }
      return { configured: true };
    }

    const signer = createWebBotAuthSigner(input);
    const now = Math.floor(Date.now() / 1000);
    if (now < signer.notBefore || now >= signer.expiresAt) {
      signer.destroy();
      throw new Error("Web Bot Auth signing key is not currently valid");
    }
    if (this.policyRevision > 0 && signer.policyRevision === undefined) {
      signer.destroy();
      throw new Error("Web Bot Auth policy changed; retry the browser operation");
    }
    if (signer.policyRevision !== undefined) {
      if (!Number.isSafeInteger(signer.policyRevision) || signer.policyRevision < 0
          || signer.policyRevision < this.policyRevision) {
        signer.destroy();
        throw new Error("Web Bot Auth policy changed; retry the browser operation");
      }
      if (signer.policyRevision > this.policyRevision) {
        this.policyRevision = signer.policyRevision;
        this.allowedProjects = null;
      }
    }
    if ((this.allowedProjects !== null && (signer.projectId === undefined
          || !this.allowedProjects.has(signer.projectId)))) {
      signer.destroy();
      throw new Error("Web Bot Auth project is not enabled by the current policy");
    }
    const attachedSession = this.sessionByTab.get(tabId);
    if (previous && previous.active && previous.keyId === signer.keyId
        && previous.signatureAgent === signer.signatureAgent
        && previous.notBefore === signer.notBefore && previous.expiresAt === signer.expiresAt) {
      previous.projectId = signer.projectId;
      previous.policyRevision = signer.policyRevision;
      signer.destroy();
      return { configured: true, kid: previous.keyId };
    }
    this.byTab.set(tabId, signer);
    if (attachedSession) {
      this.sessionByTab.set(tabId, attachedSession);
      // Keep Fetch enabled across rotation. Paused requests snapshot their
      // signer before yielding, so new requests use the replacement key.
      this.bySession.set(attachedSession, signer);
    }
    previous?.destroy();
    return { configured: true, kid: signer.keyId };
  }

  async configureIgnoredTab(tabId, input) {
    if (typeof tabId !== "string" || !tabId) throw badConfig();
    const previous = this.byTab.get(tabId);
    const sessionId = this.sessionByTab.get(tabId);
    if (sessionId) {
      this.bySession.delete(sessionId);
      this.sessionByTab.delete(tabId);
      this.unsignedSessions.delete(sessionId);
    }
    if (input === null) {
      previous?.destroy();
      this.byTab.delete(tabId);
      return { configured: true };
    }
    const signer = createWebBotAuthSigner(input);
    const now = Math.floor(Date.now() / 1000);
    if (now < signer.notBefore || now >= signer.expiresAt) {
      signer.destroy();
      previous?.destroy();
      this.byTab.delete(tabId);
      throw new Error("Web Bot Auth signing key is not currently valid");
    }
    const kid = signer.keyId;
    signer.destroy();
    previous?.destroy();
    this.byTab.delete(tabId);
    return { configured: true, kid };
  }

  async attachTab(tabId, sessionId) {
    const previousSession = this.sessionByTab.get(tabId);
    if (previousSession && previousSession !== sessionId) {
      this.unsignedSessions.add(previousSession);
      this.bySession.delete(previousSession);
      const disabled = await this.connectionProvider()?.send("Fetch.disable", {}, previousSession).then(() => true).catch(() => false);
      if (disabled) this.unsignedSessions.delete(previousSession);
    }
    this.sessionByTab.set(tabId, sessionId);
    if (this.byTab.has(tabId)) await this.activate(tabId, sessionId);
  }

  async clearTab(tabId) {
    const record = this.byTab.get(tabId);
    if (record) return this.clearRecord(tabId, record);
    const sessionId = this.sessionByTab.get(tabId);
    if (!sessionId) return;
    this.unsignedSessions.add(sessionId);
    this.bySession.delete(sessionId);
    this.sessionByTab.delete(tabId);
    const disabled = await this.connectionProvider()?.send("Fetch.disable", {}, sessionId).then(() => true).catch(() => false);
    if (disabled) this.unsignedSessions.delete(sessionId);
  }

  applyProjectPolicy(projects, reset, revision = 0) {
    if (!Array.isArray(projects) || projects.some((project) => typeof project !== "string" || !project)
        || typeof reset !== "boolean" || !Number.isSafeInteger(revision) || revision < 0) throw badConfig();
    if (revision < this.policyRevision) return;
    this.policyRevision = revision;
    const enabledProjects = new Set(projects);
    this.allowedProjects = enabledProjects;
    for (const [tabId, signer] of this.byTab) {
      if (!reset && (signer.projectId === undefined || enabledProjects.has(signer.projectId))) continue;
      signer.active = false;
      const sessionId = this.sessionByTab.get(tabId);
      if (sessionId) {
        this.bySession.delete(sessionId);
        // Fetch stays enabled so this Tab's later requests are continued
        // unsigned without affecting other projects' interception.
        this.unsignedSessions.add(sessionId);
      }
      this.byTab.delete(tabId);
      signer.destroy();
    }
  }

  async clearAll() {
    await Promise.all([...this.byTab.keys()].map((tabId) => this.clearTab(tabId)));
    this.byTab.clear();
    this.bySession.clear();
    this.sessionByTab.clear();
    this.unsignedSessions.clear();
  }

  detachSession(sessionId) {
    const signer = this.bySession.get(sessionId);
    this.bySession.delete(sessionId);
    this.unsignedSessions.delete(sessionId);
    for (const [tabId, activeSession] of this.sessionByTab) {
      if (activeSession === sessionId) {
        this.sessionByTab.delete(tabId);
        signer?.destroy();
        if (this.byTab.get(tabId) === signer) this.byTab.delete(tabId);
      }
    }
  }

  detachSessions() {
    this.bySession.clear();
    this.sessionByTab.clear();
    this.unsignedSessions.clear();
  }

  async clearRecord(tabId, record) {
    record.active = false;
    const sessionId = this.sessionByTab.get(tabId);
    if (sessionId) {
      this.unsignedSessions.add(sessionId);
      this.bySession.delete(sessionId);
      this.sessionByTab.delete(tabId);
      const disabled = await this.connectionProvider()?.send("Fetch.disable", {}, sessionId).then(() => true).catch(() => false);
      if (disabled) this.unsignedSessions.delete(sessionId);
    }
    record.destroy();
    if (this.byTab.get(tabId) === record) this.byTab.delete(tabId);
  }

  async activate(tabId, sessionId) {
    const signer = this.byTab.get(tabId);
    if (!signer?.active) return;
    const connection = this.connectionProvider();
    if (!connection) throw new Error("Web Bot Auth browser connection is unavailable");
    await connection.send("Fetch.enable", {
      patterns: [{ urlPattern: "*", requestStage: "Request" }],
    }, sessionId);
    this.unsignedSessions.delete(sessionId);
    this.bySession.set(sessionId, signer);
  }

  observe(message) {
    if (message?.method !== "Fetch.requestPaused" || typeof message.sessionId !== "string") return;
    const signer = this.bySession.get(message.sessionId);
    if (!signer && !this.unsignedSessions.has(message.sessionId)) return;
    const requestId = message.params?.requestId;
    if (typeof requestId !== "string" || !requestId) return;
    let pending = this.pendingBySession.get(message.sessionId);
    if (!pending) this.pendingBySession.set(message.sessionId, pending = new Set());
    if ([...pending].some((entry) => entry.requestId === requestId)) return;
    const entry = { requestId, done: false, promise: null };
    entry.promise = this.settlePause(message.sessionId, message.params, signer, entry)
      .catch(() => {})
      .finally(() => {
        pending.delete(entry);
        if (pending.size === 0) this.pendingBySession.delete(message.sessionId);
      });
    pending.add(entry);
  }

  async settlePause(sessionId, paused, signer, entry) {
    const connection = this.connectionProvider();
    const settle = async (method, params) => {
      if (entry.done) return;
      entry.done = true;
      await connection?.send(method, params, sessionId);
    };
    const request = paused?.request;
    let url;
    try { url = new URL(request?.url); } catch { url = null; }
    if (!url || !["http:", "https:"].includes(url.protocol)) {
      await settle("Fetch.continueRequest", { requestId: entry.requestId });
      return;
    }
    try {
      if (!signer?.active || this.bySession.get(sessionId) !== signer) {
        await settle("Fetch.continueRequest", { requestId: entry.requestId });
        return;
      }
      const headers = signWebBotAuthRequest(request, signer);
      await settle("Fetch.continueRequest", { requestId: entry.requestId, headers: cdpHeaders(request, headers) });
    } catch {
      await settle("Fetch.failRequest", { requestId: entry.requestId, errorReason: "Failed" });
    }
  }

  async waitForSession(sessionId) {
    const pending = this.pendingBySession.get(sessionId);
    if (pending?.size) await Promise.allSettled([...pending].map((entry) => entry.promise));
  }
}
