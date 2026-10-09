// MP-08/MP-10/MP-11: isolated renderer protection and capture identity fences.
import assert from "node:assert/strict";
import test from "node:test";
import { BrowserCdpClient } from "./browser-controller-cdp.mjs";
import { blackPng } from "./browser-controller-image.mjs";

const viewport = { css_width: 8, css_height: 6, device_scale_factor: 1,
  desktop_pixel_width: 8, desktop_pixel_height: 6 };

for (const scenario of ["isolated_password", "nested_password", "local_password", "foreign_password", "unprotected",
  "child_navigation", "local_navigation", "reparented", "new_child", "late_password"]) {
  test(`MP-08/MP-10/MP-11 same-tab image ${scenario}`, async () => {
    const { browser, connection, request, source } = fixture(scenario);
    if (["child_navigation", "local_navigation", "reparented", "new_child"].includes(scenario)) {
      await assert.rejects(browser.captureArtifact(request), { code: "stale_document_reference" });
    } else if (scenario === "child_unavailable") {
      await assert.rejects(browser.captureArtifact(request), { code: "browser_artifact_unavailable" });
    } else {
      const capture = await browser.captureArtifact(request);
      const protectedFrame = false; // MP-11 password fields render dots; no generic mask.
      assert.equal(capture.redaction, protectedFrame ? "full_viewport" : "none");
      assert.deepEqual(Buffer.from(capture.data_base64, "base64"), protectedFrame ? blackPng(8, 6) : source);
      assert.equal(capture.document_id, "doc-root");
      assert.equal(capture.target_id, "tab");
      assert.ok(connection.calls.filter(c => c.method === "DOMSnapshot.captureSnapshot").every(c => c.sessionId !== "foreign"), "another tab's renderer is never inspected");
    }
    const attached = connection.calls.filter(c => c.method === "Target.attachToTarget");
    const detached = connection.calls.filter(c => c.method === "Target.detachFromTarget");
    assert.equal(detached.length, attached.length, "owned temporary frame sessions settle on success and failure");
  });
}

function fixture(scenario) {
  let captured = false;
  const calls = [];
  // A distinct synthetic CDP image lets the test prove exact source vs mask bytes.
  const source = Buffer.concat([blackPng(8, 6), Buffer.from("synthetic-CDP-source")]);
  const frame = (id, parentId, loaderId = `doc-${id}`, childFrames = []) => ({ frame: { id, parentId, loaderId }, childFrames });
  const connection = {
    browserInstanceId: "test-browser", calls,
    async send(method, params = {}, sessionId) {
      calls.push({ method, params, sessionId });
      if (method === "Page.getFrameTree") {
        if (sessionId === "root") return { frameTree: frame("root", undefined, "doc-root", [frame("local", "root", captured && scenario === "local_navigation" ? "changed" : "doc-local"), ...(captured && scenario === "new_child" ? [frame("new", "root")] : [])]) };
        return { frameTree: frame(sessionId, captured && scenario === "reparented" && sessionId === "child" ? "foreign-root" : ({ child: "root", nested: "child", foreign: "foreign-root" })[sessionId], captured && scenario === "child_navigation" && sessionId === "child" ? "changed" : `doc-${sessionId}`) };
      }
      if (method === "Page.getLayoutMetrics") return { cssVisualViewport: { pageX: 0, pageY: 0, clientWidth: 8, clientHeight: 6, scale: 1 } };
      if (method === "Target.getTargets") return { targetInfos: ["child", "nested", "foreign"].map(targetId => ({ type: "iframe", targetId })) };
      if (method === "Target.attachToTarget") return { sessionId: params.targetId };
      if (method === "DOMSnapshot.captureSnapshot") {
        if (scenario === "child_unavailable" && sessionId === "child") throw new Error("renderer disconnected");
        const password = (sessionId === "child" && scenario === "isolated_password") || (sessionId === "nested" && scenario === "nested_password")
          || (sessionId === "root" && scenario === "local_password") || (sessionId === "foreign" && scenario === "foreign_password")
          || (captured && sessionId === "child" && scenario === "late_password");
        return { strings: ["type", "password"], documents: [{ nodes: { attributes: password ? [[0, 1]] : [] } }] };
      }
      if (method === "Page.captureScreenshot") { captured = true; return { data: source.toString("base64") }; }
      return {};
    },
  };
  const browser = new BrowserCdpClient();
  browser.connection = connection; browser.browserGeneration = 1;
  browser.resolvePageTarget = async () => ({ connection, sessionId: "root" });
  browser.viewportByTarget.set("tab", JSON.stringify({ width: 8, height: 6, deviceScaleFactor: 1, mobile: false, screenWidth: 8, screenHeight: 6 }));
  return { browser, connection, source, request: { target_id: "tab", document_id: "doc-root", browser_generation: 1, viewport, kind: "image" } };
}
