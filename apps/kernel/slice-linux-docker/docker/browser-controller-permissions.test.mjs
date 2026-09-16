import assert from "node:assert/strict";
import test from "node:test";

import { setBrowserPermission } from "./browser-controller-permissions.mjs";

test("permission decisions are document-bound and scoped to the current HTTP origin", async () => {
  const connection = new FakePermissionConnection();
  const result = await setBrowserPermission({
    connection,
    sessionId: "session-a",
    targetId: "target-a",
    documentId: "loader-a",
    targetUrl: "https://app.example.test/path?query=1",
    permission: "camera",
    setting: "granted",
  });

  assert.deepEqual(result, {
    target_id: "target-a",
    document_id: "loader-a",
    permission: "camera",
    setting: "granted",
  });
  assert.deepEqual(
    connection.calls.find((call) => call.method === "Browser.setPermission"),
    {
      method: "Browser.setPermission",
      params: {
        permission: { name: "camera" },
        setting: "granted",
        origin: "https://app.example.test",
      },
      sessionId: undefined,
    },
  );
});

test("permission control rejects unsupported names, unsafe origins, and stale documents", async () => {
  const connection = new FakePermissionConnection();
  const base = {
    connection,
    sessionId: "session-a",
    targetId: "target-a",
    documentId: "loader-a",
    targetUrl: "https://app.example.test/",
    setting: "denied",
  };
  await assert.rejects(
    setBrowserPermission({ ...base, permission: "filesystem" }),
    (error) => error.code === "browser_permission_invalid",
  );
  await assert.rejects(
    setBrowserPermission({ ...base, permission: "geolocation", targetUrl: "file:///tmp/page.html" }),
    (error) => error.code === "browser_permission_origin_denied",
  );
  await assert.rejects(
    setBrowserPermission({ ...base, permission: "geolocation", setting: "allow" }),
    (error) => error.code === "browser_permission_invalid",
  );
  connection.loaderId = "loader-b";
  await assert.rejects(
    setBrowserPermission({ ...base, permission: "geolocation" }),
    (error) => error.code === "stale_document_reference",
  );
});

class FakePermissionConnection {
  constructor() {
    this.calls = [];
    this.loaderId = "loader-a";
  }

  async send(method, params = {}, sessionId) {
    this.calls.push({ method, params, sessionId });
    if (method === "Page.getFrameTree") {
      return { frameTree: { frame: { loaderId: this.loaderId } } };
    }
    if (method === "Browser.setPermission") return {};
    throw new Error(`unexpected CDP method ${method}`);
  }
}
