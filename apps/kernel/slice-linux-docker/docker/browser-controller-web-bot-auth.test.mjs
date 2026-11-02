import assert from "node:assert/strict";
import { createHash, createPrivateKey, createPublicKey, verify } from "node:crypto";
import test from "node:test";

import { BrowserCdpClient } from "./browser-controller-cdp.mjs";
import { AppTabs } from "./browser-controller-apps.mjs";
import { createWebBotAuthSigner, signWebBotAuthRequest } from "./browser-controller-web-bot-auth.mjs";

const PKCS8_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const seed = Buffer.from(Array.from({ length: 32 }, (_, index) => index + 1));
const signerInput = () => {
  const now = Math.floor(Date.now() / 1000);
  return {
    private_key: seed.toString("base64url"),
    signature_agent: "https://agents.example.test",
    not_before: now - 100,
    expires_at: now + 86_300,
  };
};

function verifySignedRequest(request, headers, publicKey) {
  const input = headers["Signature-Input"].slice("sig1=".length);
  const created = Number(input.match(/;created=(\d+)/)?.[1]);
  const expires = Number(input.match(/;expires=(\d+)/)?.[1]);
  const nonce = input.match(/;nonce="([^"]+)"/)?.[1];
  const keyid = input.match(/;keyid="([^"]+)"/)?.[1];
  const url = new URL(request.url);
  url.hash = "";
  const agent = headers["Signature-Agent"].match(/sig1="([^"]+)"/)?.[1];
  const signatureParams = input;
  const base = [
    `"@authority": ${url.host}`,
    `"@method": ${request.method}`,
    `"@target-uri": ${url.href}`,
    `"signature-agent";key="sig1": "${agent}"`,
    `"@signature-params": ${signatureParams}`,
  ].join("\n");
  const signature = Buffer.from(headers.Signature.match(/sig1=:([^:]+):/)?.[1] ?? "", "base64");
  assert.ok(Number.isSafeInteger(created));
  assert.ok(Number.isSafeInteger(expires) && expires <= created + 300);
  assert.ok(nonce);
  assert.ok(keyid);
  assert.equal(agent, "https://agents.example.test/.well-known/http-message-signatures-directory");
  assert.equal(keyid, createHash("sha256").update(JSON.stringify({ crv: "Ed25519", kty: "OKP", x: publicKey.export({ format: "jwk" }).x })).digest("base64url"));
  assert.equal(verify(null, Buffer.from(base), publicKey, signature), true);
}

test("Web Bot Auth signs request components and derives its RFC 7638 key id", () => {
  const privateKey = createPrivateKey({ key: Buffer.concat([PKCS8_PREFIX, seed]), format: "der", type: "pkcs8" });
  const publicKey = createPublicKey(privateKey);
  const signer = createWebBotAuthSigner(signerInput());
  const request = { method: "POST", url: "https://example.test/a?b=c#client-fragment", headers: { Accept: "application/json" } };
  const headers = signWebBotAuthRequest(request, signer);
  assert.match(headers["Signature-Input"], /"@authority" "@method" "@target-uri" "signature-agent";key="sig1"/);
  assert.match(headers["Signature-Input"], /;alg="ed25519";tag="web-bot-auth"$/);
  assert.doesNotMatch(headers["Signature-Input"], /client-fragment/);
  verifySignedRequest(request, headers, publicKey);
  assert.equal(signerInput().private_key.length, 43);
  signer.destroy();
  assert.ok(signer.seed.every((byte) => byte === 0));
});

test("Fetch pauses are signed asynchronously through the CDP event route", async () => {
  const sent = [];
  let releaseContinue;
  const connection = {
    isOpen: () => true,
    async send(method, params, sessionId) {
      sent.push({ method, params, sessionId });
      if (method === "Fetch.continueRequest") await new Promise((resolve) => { releaseContinue = resolve; });
      return {};
    },
  };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  browser.connection = connection;
  const config = signerInput();
  const configured = await browser.webBotAuth.configureTab("tab-a", config);
  await browser.webBotAuth.attachTab("tab-a", "session-a");
  assert.equal(config.private_key, "", "the command copy of the private seed is discarded");
  const dispatchResult = browser.recordConnectionEvent({
    method: "Fetch.requestPaused",
    sessionId: "session-a",
    params: {
      requestId: "pause-a",
      request: { method: "GET", url: "https://example.test/page", headers: { Accept: "text/html" } },
    },
  });
  assert.equal(dispatchResult, undefined, "CDP event dispatch does not await the Fetch command reply");
  assert.equal(typeof releaseContinue, "function");
  releaseContinue();
  await browser.webBotAuth.waitForSession("session-a");
  const continued = sent.find((entry) => entry.method === "Fetch.continueRequest");
  assert.equal(continued.params.requestId, "pause-a");
  const headers = Object.fromEntries(continued.params.headers.map(({ name, value }) => [name, value]));
  const privateKey = createPrivateKey({ key: Buffer.concat([PKCS8_PREFIX, seed]), format: "der", type: "pkcs8" });
  verifySignedRequest({ method: "GET", url: "https://example.test/page" }, headers, createPublicKey(privateKey));
  await browser.webBotAuth.clearAll();
  assert.equal(browser.webBotAuth.byTab.size, 0);
  assert.ok(sent.some((entry) => entry.method === "Fetch.disable"));
  assert.equal(configured.configured, true);
});

test("non-HTTP pauses continue unsigned and expired keys fail the request", async () => {
  const sent = [];
  const connection = { async send(method, params, sessionId) { sent.push({ method, params, sessionId }); return {}; } };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  browser.connection = connection;
  const config = signerInput();
  await browser.webBotAuth.configureTab("tab-b", config);
  await browser.webBotAuth.attachTab("tab-b", "session-b");
  browser.recordConnectionEvent({ method: "Fetch.requestPaused", sessionId: "session-b", params: { requestId: "data", request: { method: "GET", url: "data:text/plain,hi", headers: {} } } });
  await browser.webBotAuth.waitForSession("session-b");
  assert.equal(sent.find((entry) => entry.method === "Fetch.continueRequest").params.headers, undefined);
  browser.webBotAuth.byTab.get("tab-b").expiresAt = 1;
  browser.recordConnectionEvent({ method: "Fetch.requestPaused", sessionId: "session-b", params: { requestId: "expired", request: { method: "GET", url: "https://example.test/", headers: {} } } });
  await browser.webBotAuth.waitForSession("session-b");
  assert.ok(sent.some((entry) => entry.method === "Fetch.failRequest" && entry.params.requestId === "expired"));
  await browser.webBotAuth.clearAll();
});

test("reconfiguring the same signer is idempotent and rotation keeps Fetch enabled", async () => {
  const sent = [];
  const connection = { async send(method, params, sessionId) { sent.push({ method, params, sessionId }); return {}; } };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  browser.connection = connection;
  const first = signerInput();
  first.project_id = "project-a";
  await browser.webBotAuth.configureTab("tab-repeat", first);
  await browser.webBotAuth.attachTab("tab-repeat", "session-repeat");
  const callCount = sent.length;
  const repeated = signerInput();
  repeated.project_id = "project-b";
  const result = await browser.webBotAuth.configureTab("tab-repeat", repeated);
  assert.equal(result.kid, browser.webBotAuth.byTab.get("tab-repeat").keyId);
  assert.equal(browser.webBotAuth.byTab.get("tab-repeat").projectId, "project-b", "same-key update refreshes project ownership");
  assert.equal(sent.length, callCount, "same signer causes no Fetch CDP calls");
  assert.equal(repeated.private_key, "", "temporary seed is discarded");

  const rotated = signerInput();
  rotated.private_key = Buffer.from(Array.from({ length: 32 }, (_, index) => 255 - index)).toString("base64url");
  await browser.webBotAuth.configureTab("tab-repeat", rotated);
  assert.equal(sent.length, callCount, "rotation swaps signer without a Fetch disable/enable gap");
  assert.equal(sent.some((entry) => entry.method === "Fetch.disable"), false);
  await browser.webBotAuth.clearAll();
});

test("signer accepts only secure or loopback origins and short validity windows", () => {
  for (const signature_agent of ["http://agents.example.test", "http://192.168.1.4"]) {
    assert.throws(() => createWebBotAuthSigner({ ...signerInput(), signature_agent }), /invalid Web Bot Auth signer/);
  }
  assert.throws(() => createWebBotAuthSigner({ ...signerInput(), expires_at: signerInput().not_before + 86_401 }), /invalid Web Bot Auth signer/);
  const valid = signerInput();
  const loopback = createWebBotAuthSigner({ ...valid, signature_agent: "http://127.0.0.1" });
  loopback.destroy();
});

test("opt-out retires signing without disabling Fetch or waiting on a pending continue command", async () => {
  const sent = [];
  let releaseContinue;
  const connection = {
    async send(method, params, sessionId) {
      sent.push({ method, params, sessionId });
      if (method === "Fetch.continueRequest" && params.requestId === "inflight") await new Promise((resolve) => { releaseContinue = resolve; });
      return {};
    },
  };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  browser.connection = connection;
  const config = signerInput();
  await browser.webBotAuth.configureTab("tab-optout", config);
  await browser.webBotAuth.attachTab("tab-optout", "session-optout");
  const signer = browser.webBotAuth.byTab.get("tab-optout");
  browser.recordConnectionEvent({ method: "Fetch.requestPaused", sessionId: "session-optout", params: {
    requestId: "inflight", request: { method: "GET", url: "https://example.test/", headers: {} },
  } });
  assert.equal(typeof releaseContinue, "function");
  const optedOut = await browser.webBotAuth.configureTab("tab-optout", null);
  assert.deepEqual(optedOut, { configured: true });
  assert.equal(sent.some((entry) => entry.method === "Fetch.disable"), false);
  assert.ok(signer.seed.every((byte) => byte === 0));
  const continued = sent.find((entry) => entry.method === "Fetch.continueRequest");
  assert.ok(continued.params.headers.some(({ name }) => name === "Signature"), "in-flight request was signed before retiring the seed");
  releaseContinue();
  await browser.webBotAuth.waitForSession("session-optout");
  browser.recordConnectionEvent({ method: "Fetch.requestPaused", sessionId: "session-optout", params: {
    requestId: "after-optout", request: { method: "GET", url: "https://example.test/after", headers: {} },
  } });
  await browser.webBotAuth.waitForSession("session-optout");
  const unsigned = sent.find((entry) => entry.method === "Fetch.continueRequest" && entry.params.requestId === "after-optout");
  assert.equal(unsigned.params.headers, undefined, "the attached Fetch handler continues later requests unsigned");
  await browser.webBotAuth.clearAll();
});

test("an App tab keeps its exclusive Fetch handler when WBA is configured", async () => {
  const sent = [];
  const listeners = new Set();
  const connection = {
    isOpen: () => true,
    subscribe(listener) { listeners.add(listener); return () => listeners.delete(listener); },
    async send(method, params, sessionId) { sent.push({ method, params, sessionId }); return {}; },
    async emit(message) {
      for (const listener of listeners) listener(message);
      await new Promise((resolve) => setImmediate(resolve));
    },
  };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  browser.connection = connection;
  const appTabs = new AppTabs(browser);
  browser.appTabs = appTabs;
  appTabs.listen(connection);
  appTabs.apps.set("app-session", {
    targetId: "app-target",
    origin: "https://app.demo.invalid",
    entry: "index.html",
    assets: new Map([["index.html", { contentType: "text/html", body: Buffer.from("app").toString("base64") }]]),
  });
  connection.subscribe((message) => browser.recordConnectionEvent(message));
  browser.webBotAuth.sessionByTab.set("app-tab", "app-session");
  browser.webBotAuth.unsignedSessions.add("app-session");

  const config = signerInput();
  const configured = await browser.configureWebBotAuth("app-tab", config, "app-target");
  assert.equal(configured.configured, true);
  assert.equal(typeof configured.kid, "string");
  assert.equal(config.private_key, "", "ignored App signer seed is discarded");
  assert.equal(browser.webBotAuth.bySession.size, 0, "WBA does not claim the App-owned Fetch session");
  assert.equal(browser.webBotAuth.unsignedSessions.has("app-session"), false, "WBA does not auto-continue App-owned pauses");
  assert.equal(sent.some((entry) => entry.method === "Fetch.enable"), false);

  await connection.emit({ method: "Fetch.requestPaused", sessionId: "app-session", params: {
    requestId: "app-document",
    request: { method: "GET", url: "https://app.demo.invalid/", headers: {} },
  } });
  assert.deepEqual(sent.map((entry) => entry.method), ["Fetch.fulfillRequest"]);
  await browser.configureWebBotAuth("app-tab", null, "app-target");
  assert.equal(sent.some((entry) => entry.method === "Fetch.disable"), false, "WBA opt-out leaves App Fetch enabled");
  await browser.configureWebBotAuth("app-tab", { disabled: true, policy_revision: 1 }, "app-target");
  assert.equal(sent.some((entry) => entry.method === "Fetch.disable"), false, "disabled policy marker leaves App Fetch enabled");
  assert.equal(browser.webBotAuth.bySession.size, 0, "disabled marker does not claim the App-owned Fetch session");
  appTabs.unsubscribe?.();
});

test("project policy clears only disabled projects and continues their later requests unsigned", async () => {
  const sent = [];
  const connection = { isOpen: () => true, async send(method, params, sessionId) { sent.push({ method, params, sessionId }); return {}; } };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  browser.connection = connection;
  const first = signerInput();
  first.project_id = "project-disabled";
  const second = signerInput();
  second.project_id = "project-enabled";
  await browser.webBotAuth.configureTab("tab-disabled", first);
  await browser.webBotAuth.attachTab("tab-disabled", "session-disabled");
  await browser.webBotAuth.configureTab("tab-enabled", second);
  await browser.webBotAuth.attachTab("tab-enabled", "session-enabled");
  const retainedSigner = browser.webBotAuth.byTab.get("tab-enabled");
  const retiredSigner = browser.webBotAuth.byTab.get("tab-disabled");

  browser.webBotAuth.applyProjectPolicy(["project-enabled"], false);
  assert.equal(browser.webBotAuth.byTab.has("tab-disabled"), false);
  assert.ok(retiredSigner.seed.every((byte) => byte === 0));
  assert.equal(browser.webBotAuth.byTab.get("tab-enabled"), retainedSigner);
  assert.equal(browser.webBotAuth.bySession.get("session-enabled"), retainedSigner);
  assert.equal(browser.webBotAuth.unsignedSessions.has("session-disabled"), true);
  assert.equal(sent.some((entry) => entry.method === "Fetch.disable"), false, "project update keeps both page interceptors enabled");

  browser.recordConnectionEvent({ method: "Fetch.requestPaused", sessionId: "session-disabled", params: {
    requestId: "after-policy-disabled", request: { method: "GET", url: "https://example.test/disabled", headers: {} },
  } });
  browser.recordConnectionEvent({ method: "Fetch.requestPaused", sessionId: "session-enabled", params: {
    requestId: "after-policy-enabled", request: { method: "GET", url: "https://example.test/enabled", headers: {} },
  } });
  await Promise.all([browser.webBotAuth.waitForSession("session-disabled"), browser.webBotAuth.waitForSession("session-enabled")]);
  const continued = sent.filter((entry) => entry.method === "Fetch.continueRequest");
  const disabled = continued.find((entry) => entry.params.requestId === "after-policy-disabled");
  const enabled = continued.find((entry) => entry.params.requestId === "after-policy-enabled");
  assert.equal(disabled.params.headers, undefined, "disabled project request is continued unsigned");
  assert.ok(enabled.params.headers.some(({ name }) => name === "Signature"), "enabled project request stays signed");
  assert.equal(sent.some((entry) => entry.method === "Fetch.disable" && entry.sessionId === "session-enabled"), false);

  await browser.webBotAuth.clearTab("tab-disabled");
  await browser.webBotAuth.clearTab("tab-enabled");
});

test("stale signer material cannot re-enable a project after a newer policy revision", async () => {
  const sent = [];
  const connection = { isOpen: () => true, async send(method, params, sessionId) { sent.push({ method, params, sessionId }); return {}; } };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  browser.connection = connection;
  browser.webBotAuth.applyProjectPolicy(["project-a"], false, 4);
  const stale = signerInput();
  stale.project_id = "project-a";
  stale.policy_revision = 3;
  await assert.rejects(browser.webBotAuth.configureTab("stale-tab", stale), /policy changed/);
  assert.equal(browser.webBotAuth.byTab.has("stale-tab"), false);
  assert.equal(sent.some(({ method }) => method === "Fetch.enable"), false);

  const disabled = signerInput();
  disabled.project_id = "project-b";
  disabled.policy_revision = 4;
  await assert.rejects(browser.webBotAuth.configureTab("disabled-tab", disabled), /project is not enabled/);
  assert.equal(browser.webBotAuth.byTab.has("disabled-tab"), false);
  await browser.webBotAuth.clearAll();
});

test("stale disabled marker cannot clear a signer installed at a newer policy revision", async () => {
  const sent = [];
  const connection = { isOpen: () => true, async send(method, params, sessionId) { sent.push({ method, params, sessionId }); return {}; } };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  browser.connection = connection;
  browser.webBotAuth.applyProjectPolicy(["project-a"], false, 8);
  const active = signerInput();
  active.project_id = "project-a";
  active.policy_revision = 8;
  await browser.webBotAuth.configureTab("tab-disabled-race", active);
  await browser.webBotAuth.attachTab("tab-disabled-race", "session-disabled-race");
  const currentSigner = browser.webBotAuth.byTab.get("tab-disabled-race");
  const disableCount = sent.filter(({ method }) => method === "Fetch.disable").length;

  await assert.rejects(browser.webBotAuth.configureTab("tab-disabled-race", { disabled: true, policy_revision: 7 }), /policy changed/);
  assert.equal(browser.webBotAuth.byTab.get("tab-disabled-race"), currentSigner);
  assert.equal(browser.webBotAuth.bySession.get("session-disabled-race"), currentSigner);
  assert.equal(sent.filter(({ method }) => method === "Fetch.disable").length, disableCount);

  assert.deepEqual(await browser.webBotAuth.configureTab("tab-disabled-race", { disabled: true, policy_revision: 9 }), { configured: true });
  assert.equal(browser.webBotAuth.byTab.has("tab-disabled-race"), false);
  assert.equal(browser.webBotAuth.bySession.has("session-disabled-race"), false);
  assert.ok(browser.webBotAuth.unsignedSessions.has("session-disabled-race"), "Fetch stays enabled for unsigned continuation without a disable gap");
  assert.equal(sent.filter(({ method }) => method === "Fetch.disable").length, disableCount);
  assert.deepEqual(await browser.webBotAuth.configureTab("new-tab", { disabled: true, policy_revision: 10 }), { configured: true },
    "a new controller accepts the current disabled marker without key material");
  await browser.webBotAuth.clearAll();
});

test("a newer disabled marker retires signers on every tab before project policy RPC arrives", async () => {
  const sent = [];
  const connection = { isOpen: () => true, async send(method, params, sessionId) { sent.push({ method, params, sessionId }); return {}; } };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  browser.connection = connection;
  for (const [tabId, sessionId, projectId] of [
    ["tab-policy-new", "session-policy-new", "project-new"],
    ["tab-policy-other", "session-policy-other", "project-other"],
  ]) {
    const signer = signerInput();
    signer.project_id = projectId;
    signer.policy_revision = 1;
    await browser.webBotAuth.configureTab(tabId, signer);
    await browser.webBotAuth.attachTab(tabId, sessionId);
  }
  const oldOtherSigner = browser.webBotAuth.byTab.get("tab-policy-other");

  await browser.webBotAuth.configureTab("tab-policy-new", { disabled: true, policy_revision: 2 });
  assert.equal(browser.webBotAuth.byTab.size, 0);
  assert.ok(oldOtherSigner.seed.every((byte) => byte === 0));
  assert.equal(browser.webBotAuth.bySession.size, 0);
  assert.ok(browser.webBotAuth.unsignedSessions.has("session-policy-other"));
  browser.recordConnectionEvent({ method: "Fetch.requestPaused", sessionId: "session-policy-other", params: {
    requestId: "revision-disabled-other", request: { method: "GET", url: "https://example.test/other", headers: {} },
  } });
  await browser.webBotAuth.waitForSession("session-policy-other");
  const continued = sent.find((entry) => entry.method === "Fetch.continueRequest" && entry.params.requestId === "revision-disabled-other");
  assert.equal(continued.params.headers, undefined, "the other project's request cannot use its pre-revision key");
  await browser.webBotAuth.clearAll();
});

test("enabled material at a newer policy revision retires old tab signers before installing the new one", async () => {
  const sent = [];
  const connection = { isOpen: () => true, async send(method, params, sessionId) { sent.push({ method, params, sessionId }); return {}; } };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  browser.connection = connection;
  for (const [tabId, sessionId, projectId] of [
    ["tab-enabled-new", "session-enabled-new", "project-new"],
    ["tab-enabled-old", "session-enabled-old", "project-old"],
  ]) {
    const signer = signerInput();
    signer.project_id = projectId;
    signer.policy_revision = 1;
    await browser.webBotAuth.configureTab(tabId, signer);
    await browser.webBotAuth.attachTab(tabId, sessionId);
  }
  const oldSigner = browser.webBotAuth.byTab.get("tab-enabled-old");
  const next = signerInput();
  next.project_id = "project-new";
  next.policy_revision = 2;
  await browser.webBotAuth.configureTab("tab-enabled-new", next);
  assert.equal(browser.webBotAuth.byTab.get("tab-enabled-new").policyRevision, 2);
  assert.equal(browser.webBotAuth.byTab.has("tab-enabled-old"), false);
  assert.ok(oldSigner.seed.every((byte) => byte === 0));
  assert.ok(browser.webBotAuth.unsignedSessions.has("session-enabled-old"));
  browser.recordConnectionEvent({ method: "Fetch.requestPaused", sessionId: "session-enabled-old", params: {
    requestId: "revision-enabled-old", request: { method: "GET", url: "https://example.test/old", headers: {} },
  } });
  await browser.webBotAuth.waitForSession("session-enabled-old");
  const continued = sent.find((entry) => entry.method === "Fetch.continueRequest" && entry.params.requestId === "revision-enabled-old");
  assert.equal(continued.params.headers, undefined, "the old tab is unsigned after newer material advances revision");
  await browser.webBotAuth.clearAll();
});

test("legacy null opt-out has no asynchronous gap before a replacement signer is installed", async () => {
  const sent = [];
  const connection = { isOpen: () => true, async send(method, params, sessionId) { sent.push({ method, params, sessionId }); return {}; } };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  browser.connection = connection;
  await browser.webBotAuth.configureTab("tab-legacy-race", signerInput());
  await browser.webBotAuth.attachTab("tab-legacy-race", "session-legacy-race");
  const optingOut = browser.webBotAuth.configureTab("tab-legacy-race", null);
  assert.equal(browser.webBotAuth.byTab.has("tab-legacy-race"), false, "null clears signing synchronously");
  await browser.webBotAuth.configureTab("tab-legacy-race", signerInput());
  const replacement = browser.webBotAuth.byTab.get("tab-legacy-race");
  await optingOut;
  assert.equal(browser.webBotAuth.byTab.get("tab-legacy-race"), replacement);
  assert.equal(sent.some(({ method }) => method === "Fetch.disable"), false, "the session remains attached with no disable/re-enable race");
  await browser.webBotAuth.clearAll();
});

test("non-loopback HTTP and credential-bearing targets are rejected; non-HTTP stays unsigned", () => {
  const signer = createWebBotAuthSigner(signerInput());
  assert.throws(() => signWebBotAuthRequest({ method: "GET", url: "http://example.test/private" }, signer), /HTTPS/);
  assert.throws(() => signWebBotAuthRequest({ method: "GET", url: "https://user:pass@example.test/private" }, signer), /credentials/);
  assert.equal(signWebBotAuthRequest({ method: "GET", url: "chrome-extension://extension-id/page" }, signer), null);
  signer.destroy();
});
