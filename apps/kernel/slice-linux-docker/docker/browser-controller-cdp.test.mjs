import assert from "node:assert/strict";
import { BrowserMemory } from "./browser-memory.mjs";
import test from "node:test";

import {
  assertPrivateDebuggerUrl,
  BrowserCdpClient,
  CdpConnection,
} from "./browser-controller-cdp.mjs";
import { handleBrowserControllerRequest } from "./browser-controller.mjs";

const viewport = {
  css_width: 1280,
  css_height: 720,
  device_scale_factor: 2,
  desktop_pixel_width: 2560,
  desktop_pixel_height: 1440,
};

test("MP-08/MP-10 page reconciliation overlaps independent focus and frame reads", async () => {
  const connection = new FakeConnection(), ready = Promise.withResolvers();
  const send = connection.send.bind(connection);
  connection.send = async (method, params, sessionId) => {
    const value = await send(method, params, sessionId);
    if (method === "Runtime.evaluate" && params.expression === "document.visibilityState === 'visible'") await ready.promise;
    return value;
  };
  const browser = new BrowserCdpClient({connectionFactory:async()=>connection});
  await browser.ensureConnection();
  const inspected = browser.inspectPage(connection, {targetId:"target-a"}, {});
  try {
    for (let n=0;n<20;n++) await new Promise(setImmediate);
    assert.equal(connection.calls.filter(c=>c.method==="Page.getFrameTree").length,2,
      "frame registration must progress while the renderer focus reply is pending");
  } finally { ready.resolve(); await inspected; await browser.close(); }
});

test("MP-08/MP-10/MP-11 failed focus drains pending frame observation before returning", async () => {
  const connection = new FakeConnection(), ready = Promise.withResolvers();
  const send = connection.send.bind(connection); let frames=0,settled=false;
  connection.send = async (method, params, sessionId) => {
    const value = await send(method, params, sessionId);
    if (method === "Page.getFrameTree" && ++frames === 2) await ready.promise;
    if (method === "Runtime.evaluate" && params.expression === "document.visibilityState === 'visible'") throw Error("Execution context was destroyed");
    return value;
  };
  const browser = new BrowserCdpClient({connectionFactory:async()=>connection});
  await browser.ensureConnection();
  const inspected = browser.inspectPage(connection,{targetId:"target-a"},{}).then(
    ()=>{settled=true;assert.fail("stale focus must fail closed");},
    error=>{settled=true;assert.equal(error.code,"stale_document_reference");});
  try {
    for (let n=0;n<20;n++) await new Promise(setImmediate);
    assert.equal(settled,false,"a rejected focus read must not leave frame work behind the caller");
  } finally { ready.resolve();await inspected;await browser.close(); }
  assert.equal(browser.documentIdsByTarget.has("target-a"),false);
});

test("concurrent reads share connection initialization", async () => {
  const ready = Promise.withResolvers();
  const connection = new FakeConnection();
  let opens = 0;
  const browser = new BrowserCdpClient({ connectionFactory: async () => {
    opens += 1;
    await ready.promise;
    return connection;
  } });
  const first = browser.ensureConnection();
  const second = browser.ensureConnection();
  ready.resolve();
  const connections = await Promise.all([first, second]);
  assert.equal(opens, 1);
  assert.equal(connections[0], connections[1]);
});

test("concurrent reads share fully initialized target sessions", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.ensureConnection();
  const [first, second] = await Promise.all([
    browser.ensureTargetSession(connection, "target-a"),
    browser.ensureTargetSession(connection, "target-a"),
  ]);
  assert.equal(first, second);
  assert.equal(connection.calls.filter(call => call.method === "Target.attachToTarget").length, 1);
  assert.equal(connection.calls.filter(call => call.method === "Page.enable").length, 1);
});

test("MP-08/MP-10: page and worker response retention fits the bounded body reader", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  try {
    await browser.ensureConnection();
    await browser.ensureTargetSession(connection, "target-a");
    await browser.ensureWriterTargetSession(connection, "worker-a");
    const enabled = connection.calls.filter(call => call.method === "Network.enable");
    assert.equal(enabled.length, 2);
    for (const call of enabled) {
      assert.equal(call.params.maxResourceBufferSize, 4 * 1024 * 1024);
      assert.equal(call.params.maxTotalBufferSize, 8 * 1024 * 1024);
    }
  } finally { await browser.close(); }
});

test("MP-08/MP-10: a headed Room paints the full physical desktop at source DPR2", async () => {
  for (const headed of [false, true]) {
    const connection = new FakeConnection();
    const browser = new BrowserCdpClient({ connectionFactory: async () => connection,
      ...(headed ? { applyCanonicalDisplay: async () => {} } : {}) });
    try {
      await browser.reconcile({ ...viewport, device_scale_factor: 2, desktop_pixel_width: 2560, desktop_pixel_height: 1600 });
      const metrics = connection.calls.find(call => call.method === 'Emulation.setDeviceMetricsOverride').params;
      assert.equal(metrics.deviceScaleFactor, 2);
      assert.equal(metrics.scale, headed ? 2 : undefined, 'a 1x X desktop must paint the emulated 2x view at physical size');
    } finally { await browser.close(); }
  }
});

test("MP-08/MP-10: headed visible size follows page metrics through reconcile and layout", async () => {
  for (const headed of [false, true]) for (const dpr of [1, 2]) {
    const connection = new FakeConnection();
    connection.closedTargetIds.add('target-b');
    const browser = new BrowserCdpClient({ connectionFactory: async () => connection,
      ...(headed ? { applyCanonicalDisplay: async () => {} } : {}) });
    try {
      await browser.reconcile({ ...viewport, device_scale_factor: dpr,
        desktop_pixel_width: 1280 * dpr, desktop_pixel_height: 720 * dpr });
      const visible = () => connection.calls.filter(call => call.method === 'Emulation.setVisibleSize');
      assert.deepEqual(visible().map(call => call.params), headed ? [{ width: 1280 * dpr, height: 720 * dpr }] : []);
      await browser.setLayout('target-a', { width: 640, height: 480 });
      assert.deepEqual(visible().map(call => call.params), headed
        ? [{ width: 1280 * dpr, height: 720 * dpr }, { width: 640 * dpr, height: 480 * dpr }] : []);
      for (const call of visible()) {
        const index = connection.calls.indexOf(call);
        assert.equal(connection.calls[index - 1].method, 'Emulation.setDeviceMetricsOverride');
        assert.equal(call.sessionId, connection.calls[index - 1].sessionId);
      }
    } finally { await browser.close(); }
  }
});

test("MP-08/MP-10/MP-11: headed page bottom fits below the toolbar at every source density", async () => {
  for (const [width, height] of [[1280, 800], [1920, 1080]]) for (const dpr of [1, 2]) for (const oversized of [false, true]) {
    const connection = new FakeConnection(), send = connection.send.bind(connection);
    connection.closedTargetIds.add('target-b');
    let toolbar = 87;
    connection.send = async (method, params, sessionId) => {
      const result = await send(method, params, sessionId);
      if (method === 'Browser.getWindowForTarget') return { windowId: 1, bounds: { left: 0, top: 0, width: width * dpr, height: height * dpr + (oversized ? toolbar : 0) } };
      if (method === 'Browser.getWindowBounds') return { bounds: { windowState: toolbar ? 'maximized' : 'fullscreen' } };
      if (method === 'Runtime.evaluate' && params.expression === '({ width: innerWidth, height: innerHeight })') return { result: { value: {
        width: width * dpr, height: height * dpr - (oversized ? 0 : toolbar),
      } } };
      return result;
    };
    const browser = new BrowserCdpClient({ connectionFactory: async () => connection, applyCanonicalDisplay: async () => {} });
    const source = { css_width: width, css_height: height, device_scale_factor: dpr,
      desktop_pixel_width: width * dpr, desktop_pixel_height: height * dpr };
    const bottomFits = () => {
      const metrics = connection.calls.filter(c => c.method === 'Emulation.setDeviceMetricsOverride').at(-1).params;
      const visible = connection.calls.filter(c => c.method === 'Emulation.setVisibleSize').at(-1).params;
      assert.ok(toolbar + metrics.height * dpr <= source.desktop_pixel_height,
        `page bottom must remain on the desktop below ${toolbar}px toolbar at DPR${dpr}`);
      assert.equal(visible.height, Math.round(metrics.height * dpr));
      assert.equal(visible.width, Math.round(metrics.width * dpr));
    };
    try {
      await browser.reconcile(source, { browserBarVisible: true });
      bottomFits();
      await browser.setLayout('target-a', { width: width - 100, height });
      bottomFits();
      toolbar = 0;
      await browser.reconcile(source, { browserBarVisible: false });
      bottomFits();
      assert.equal(connection.calls.filter(c => c.method === 'Emulation.setDeviceMetricsOverride').at(-1).params.height, height);
    } finally { await browser.close(); }
  }
});

test("MP-08/MP-10: memory pressure refuses navigation before dispatch and still permits closing a tab", async () => {
  const connection = new FakeConnection();
  const memory = new BrowserMemory({ availableMemory: () => 128 * 1024 * 1024 });
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection, memory });
  try {
    await browser.reconcile(viewport);
    await assert.rejects(browser.navigate({ target_id: "target-a", document_id: "loader-a", url: "https://next.test/" }),
      error => error.code === 'browser_memory_pressure');
    assert.equal(connection.calls.filter(call => call.method === 'Page.navigate').length, 0);
    await browser.manageTab({ action: 'close', target_id: 'target-a', document_id: 'loader-a' });
    assert.equal(connection.calls.filter(call => call.method === 'Target.closeTarget').length, 1);
  } finally { await browser.close(); }
});

test("persistent browser connection returns page identities, focus, and applied viewport", async () => {
  const connection = new FakeConnection();
  let connectionCount = 0;
  const browser = new BrowserCdpClient({
    connectionFactory: async () => {
      connectionCount += 1;
      return connection;
    },
  });

  const first = await browser.reconcile(viewport);
  const second = await browser.reconcile(viewport);

  assert.equal(connectionCount, 1);
  assert.equal(first.browser_generation, 1);
  assert.equal(first.event_cursor, 1);
  assert.deepEqual(first, second);
  assert.deepEqual(first.tabs, [
    {
      target_id: "target-a",
      document_id: "loader-a",
      url: "https://a.test/",
      title: "A",
    },
    {
      target_id: "target-b",
      document_id: "loader-b",
      url: "https://b.test/",
      title: "B",
    },
  ]);
  assert.equal(first.focused_target_id, "target-b");
  assert.equal(connection.calls.filter(call => call.method === "Emulation.setDeviceMetricsOverride").length, 2,
    "MP-08/MP-10 unchanged reconciliation must not resize either tab again");
  assert.deepEqual(first.viewport, viewport);
  assert.equal(
    connection.calls.filter((call) => call.method === "Target.attachToTarget").length,
    3,
    "persistent page and cookie-writing worker sessions should be reused",
  );
  assert.equal(
    connection.calls.filter((call) => call.method === "Page.enable").length,
    2,
    "each persistent target session should subscribe to Page lifecycle events once",
  );
  for (const method of [
    "Page.setLifecycleEventsEnabled",
    "Runtime.enable",
    "Inspector.enable",
  ]) {
    assert.equal(
      connection.calls.filter((call) => call.method === method).length,
      2,
      `${method} should run once for each persistent target session`,
    );
  }
  assert.equal(
    connection.calls.filter((call) => call.method === "Network.enable").length,
    3,
    "network requests must be tracked for pages and cookie-writing workers",
  );
  assert.deepEqual(
    connection.calls.find(
      (call) => call.method === "Emulation.setDeviceMetricsOverride" && call.sessionId === "session-a",
    ).params,
    {
      width: 1280,
      height: 720,
      deviceScaleFactor: 2,
      mobile: false,
      screenWidth: 1280,
      screenHeight: 720,
    },
  );
  assert.equal(
    connection.calls.find(
      (call) => call.method === "Runtime.evaluate" && call.sessionId === "session-a",
    ).params.expression,
    "document.visibilityState === 'visible'",
    "active-tab focus must not disappear merely because the browser window loses OS focus",
  );
});

test("MP-08/MP-10: a crashed renderer is reloaded once without replaying input or replacing another tab", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  try {
    await browser.reconcile(viewport);
    connection.emit({ method: 'Inspector.targetCrashed', sessionId: 'session-a', params: {} });
    connection.emit({ method: 'Target.targetCrashed', params: { targetId: 'target-a', status: 'oom', errorCode: 9 } });
    await assert.rejects(browser.snapshot({ target_id: 'target-a', document_id: 'loader-a' }),
      error => error.code === 'browser_renderer_crashed');
    await new Promise(resolve => setTimeout(resolve, 1200));
    const reloads = connection.calls.filter(call => call.method === 'Page.reload');
    assert.equal(reloads.length, 1);
    assert.equal(reloads[0].sessionId, 'session-a');
    assert.equal(connection.calls.filter(call => call.method === 'Input.dispatchKeyEvent' || call.method === 'Target.closeTarget').length, 0);
  } finally { await browser.close(); }
});

test("focus is read in a controller-owned isolated world a page cannot redefine", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);
  await browser.reconcile(viewport);
  const worlds = connection.calls.filter((call) => call.method === "Page.createIsolatedWorld");
  assert.deepEqual(
    worlds.map((call) => [call.sessionId, call.params.frameId, call.params.worldName]),
    [["session-a", "frame-a", "chariox-controller-focus"], ["session-b", "frame-b", "chariox-controller-focus"]],
    "one world per document, reused by later polls",
  );
  const focusReads = connection.calls.filter(
    (call) => call.method === "Runtime.evaluate" && call.params.expression === "document.visibilityState === 'visible'",
  );
  assert.equal(focusReads.length, 4);
  assert.ok(focusReads.every((call) => typeof call.params.contextId === "number"), "never evaluated in the page's own world");
});

test("a Tab that closes while it is inspected drops out instead of failing the reconcile", async () => {
  const connection = new FakeConnection();
  connection.vanishOnInspect = { "session-a": "target-a" };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  const reconciled = await browser.reconcile(viewport);
  assert.deepEqual(reconciled.tabs.map((tab) => tab.target_id), ["target-b"]);
  assert.equal(reconciled.focused_target_id, "target-b");
});

// MP-08/MP-10/MP-11: BBC workers can end between discovery and attachment.
for (const type of ["worker", "shared_worker"]) {
 test(`MP-10 an exited ${type} cannot fail ordinary page reconciliation`, async()=>{
  const connection=new FakeConnection();connection.includeWorker=false;
  connection.extraTargets=[{targetId:'transient-writer',type}];
  const send=connection.send.bind(connection);
  connection.send=async(method,params,session)=>{
   if(method==='Target.attachToTarget'&&params.targetId==='transient-writer'){
    connection.closedTargetIds.add('transient-writer');
    throw new Error('Target.attachToTarget: No target with given id found');
   }
   return send(method,params,session);
  };
  const browser=new BrowserCdpClient({connectionFactory:async()=>connection});
  const result=await browser.reconcile(viewport);
  assert.deepEqual(result.tabs.map(t=>t.target_id),['target-a','target-b']);
  assert.equal(browser.sessionsByTarget.has('transient-writer'),false);
  // Security-critical import still independently fences all live writers.
  connection.includeWorker=true;await browser.withCookieWritersQuiesced(async()=>{});
  assert(connection.calls.some(c=>c.method==='Runtime.evaluate'&&c.params.expression==='self.close()'&&c.sessionId==='worker-session-a'));
 });
}
for (const persistent of [false, true]) {
 test(`MP-11 a still-listed writer ${persistent ? 'keeps its failure' : 'reattaches its stale session'}`,async()=>{
  const connection=new FakeConnection(),send=connection.send.bind(connection);let attempts=0;
  connection.send=async(method,params,session)=>{
   if(method==='Target.attachToTarget'&&params.targetId==='worker-a'){
    attempts++;if(persistent||attempts===1)throw new Error('Target.attachToTarget: No target with given id found');
   }
   return send(method,params,session);
  };
  const browser=new BrowserCdpClient({connectionFactory:async()=>connection});
  if(persistent)await assert.rejects(browser.reconcile(viewport),/No target with given id found/);
  else {await browser.reconcile(viewport);assert.equal(browser.sessionsByTarget.get('worker-a'),'worker-session-a');}
  assert.equal(attempts,2);
 });
}
test('MP-11 live writer attachment failures remain fail closed',async()=>{
 const connection=new FakeConnection(),send=connection.send.bind(connection);
 connection.send=async(method,params,session)=>{
  if(method==='Target.attachToTarget'&&params.targetId==='worker-a')throw new Error('Target.attachToTarget: permission denied');
  return send(method,params,session);
 };
 const browser=new BrowserCdpClient({connectionFactory:async()=>connection});
 await assert.rejects(browser.reconcile(viewport),/permission denied/);
});

test("viewport validation happens before opening a browser connection", async () => {
  let connected = false;
  const browser = new BrowserCdpClient({
    connectionFactory: async () => {
      connected = true;
      return new FakeConnection();
    },
  });

  await assert.rejects(
    browser.reconcile({ ...viewport, css_width: 0 }),
    (error) => error.code === "browser_viewport_invalid",
  );
  assert.equal(connected, false);
});

test("cookie writer fence drains an existing worker response before import", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);
  connection.emit({
    method: "Network.requestWillBeSent",
    sessionId: "worker-session-a",
    params: { requestId: "worker-cookie-response" },
  });
  let imported = false;
  const guardedImport = browser.withCookieWritersQuiesced(async () => { imported = true; });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(imported, false, "a worker response remains a cookie writer until it settles");
  connection.emit({
    method: "Network.loadingFinished",
    sessionId: "worker-session-a",
    params: { requestId: "worker-cookie-response" },
  });
  await guardedImport;
  assert.equal(imported, true);
});

test("cookie writer fence closes a worker first discovered after reconcile through its session", async () => {
  const connection = new FakeConnection();
  connection.includeWorker = false;
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);
  connection.includeWorker = true;

  await browser.withCookieWritersQuiesced(async () => {});

  assert.equal(connection.calls.some(({ method, params }) =>
    method === "Target.attachToTarget" && params.targetId === "worker-a"), true);
  assert.equal(connection.calls.some(({ method, params, sessionId }) =>
    method === "Runtime.evaluate" && params.expression === "self.close()"
      && sessionId === "worker-session-a"), true);
  assert.equal(connection.calls.some(({ method, params }) =>
    method === "Target.closeTarget" && params.targetId === "worker-a"), false);
});

test("failed event subscription closes the connection before a clean reconnect", async () => {
  const failed = new FakeConnection();
  failed.failDiscovery = true;
  const recovered = new FakeConnection();
  const connections = [failed, recovered];
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connections.shift(),
  });

  await assert.rejects(browser.reconcile(viewport), /discovery failed/);
  assert.equal(failed.open, false);
  const result = await browser.reconcile(viewport);

  assert.equal(result.browser_generation, 2);
  assert.equal(result.event_cursor, 1);
});

test("App pages get a narrower viewport; the panel takes the rest", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  browser.appTabs = { apps: new Map([["session-a", { targetId: "target-a" }]]) };
  const widths = () => new Map(connection.calls
    .filter((call) => call.method === "Emulation.setDeviceMetricsOverride")
    .map((call) => [call.sessionId, call.params.width]));
  await browser.reconcile(viewport, { appPanelCssWidth: 380 });
  assert.equal(widths().get("session-a"), 900);
  // A page the kernel sized (bottom panel) keeps its width and loses height.
  browser.appTabs.apps.get("session-a").page = { width: 1280, height: 460 };
  await browser.reconcile(viewport, { appPanelCssWidth: 380 });
  const last = connection.calls.filter((call) => call.method === "Emulation.setDeviceMetricsOverride" && call.sessionId === "session-a").at(-1).params;
  assert.deepEqual([last.width, last.height], [1280, 460]);
  // A size larger than the viewport is ignored.
  browser.appTabs.apps.get("session-a").page = { width: 5000, height: 460 };
  await browser.reconcile(viewport, { appPanelCssWidth: 380 });
  assert.equal(widths().get("session-a"), 900);
  browser.appTabs.apps.get("session-a").page = null;
  assert.ok([...widths()].every(([session, width]) => session === "session-a" || width === 1280));
  assert.equal(browser.appMetrics().width, 900);
  // An older kernel sends no width, and a width that leaves no page is refused.
  for (const appPanelCssWidth of [undefined, 1280]) {
    await browser.reconcile(viewport, { appPanelCssWidth });
    assert.equal(widths().get("session-a"), 1280);
    assert.equal(browser.appMetrics(), null);
  }
});

test("a reconnected browser applies the bar to windows whose ids an earlier browser used", async () => {
  // Both browsers put target-a in window 1; the second starts it clipped.
  const windowed = (state) => {
    const connection = new FakeConnection();
    connection.includeWorker = false;
    connection.windows = { 1: state };
    const send = connection.send.bind(connection);
    connection.send = async (method, params = {}, sessionId) => {
      if (method === "Browser.getWindowForTarget") return { windowId: params.targetId === "target-a" ? 1 : 2 };
      if (method === "Browser.getWindowBounds") return { bounds: { windowState: connection.windows[params.windowId] ?? "fullscreen" } };
      if (method === "Browser.setWindowBounds") connection.windows[params.windowId] = params.bounds.windowState;
      return send(method, params, sessionId);
    };
    return connection;
  };
  const first = windowed("normal");
  const second = windowed("normal");
  const connections = [first, second];
  const browser = new BrowserCdpClient({ connectionFactory: async () => connections.shift() });
  await browser.reconcile(viewport, { browserBarVisible: false });
  assert.equal(first.windows[1], "fullscreen");
  first.open = false; // Chromium was relaunched.
  await browser.reconcile(viewport, { browserBarVisible: false });
  assert.equal(second.windows[1], "fullscreen");
});

test("a failed cookie-writer fence release is never reused", async () => {
  const connection = new ReleaseFaultConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);
  const initialFenceArmCount = connection.calls.filter(({ method, params }) =>
    method === "Target.setAutoAttach" && params.autoAttach === true).length;

  await assert.rejects(
    browser.withCookieWritersQuiesced(async () => {}),
    { code: "browser_cookie_writer_fence_release_failed", recoveryRequired: true },
  );
  connection.failRelease = false;
  await browser.withCookieWritersQuiesced(async () => {});

  assert.equal(connection.calls.filter(({ method, params }) =>
    method === "Target.setAutoAttach" && params.autoAttach === true).length,
  initialFenceArmCount + 2);
});

test("a failed cookie-writer fence acquisition is never reused", async () => {
  const connection = new AcquisitionFaultConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);
  const initialFenceArmCount = connection.calls.filter(({ method, params }) =>
    method === "Target.setAutoAttach" && params.autoAttach === true).length;

  await assert.rejects(
    browser.withCookieWritersQuiesced(async () => {}),
    { code: "browser_cookie_writer_fence_failed", recoveryRequired: true },
  );
  connection.failAcquisition = false;
  await browser.withCookieWritersQuiesced(async () => {});

  assert.equal(connection.calls.filter(({ method, params }) =>
    method === "Target.setAutoAttach" && params.autoAttach === true).length,
  initialFenceArmCount + 2);
});

test("a detached target session is discarded before the next reconcile", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);

  connection.emit({
    method: "Target.detachedFromTarget",
    sessionId: "session-a",
    params: { sessionId: "session-a", targetId: "target-a" },
  });
  await browser.reconcile(viewport);

  assert.equal(
    connection.calls.filter(
      (call) => call.method === "Target.attachToTarget" && call.params.targetId === "target-a",
    ).length,
    2,
  );
});

test("tab lifecycle operations stay document-bound and use browser target commands", async () => {
  const connection = new DelayedTargetCloseConnection(1);
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);

  const activated = await browser.manageTab({
    target_id: "target-a",
    document_id: "loader-a",
    action: "activate",
  });
  const closed = await browser.manageTab({
    target_id: "target-a",
    document_id: "loader-a",
    action: "close",
  });

  assert.deepEqual(activated, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    action: "activate",
  });
  assert.deepEqual(closed, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    action: "close",
  });
  assert.deepEqual(
    connection.calls.find((call) => call.method === "Page.bringToFront"),
    { method: "Page.bringToFront", params: {}, sessionId: "session-a" },
  );
  assert.deepEqual(
    connection.calls.find((call) => call.method === "Target.closeTarget")?.params,
    { targetId: "target-a" },
  );
  await assert.rejects(
    browser.manageTab({ target_id: "target-b", document_id: "loader-old", action: "activate" }),
    (error) => error.code === "stale_document_reference",
  );
  await assert.rejects(
    browser.manageTab({ target_id: "target-b", document_id: "loader-b", action: "detach" }),
    (error) => error.code === "browser_tab_action_invalid",
  );
});

test("tab activation brings the selected page to front before focus is reconciled", async () => {
  const connection = new PageActivationConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  const before = await browser.reconcile(viewport);
  assert.equal(before.focused_target_id, "target-b");

  await browser.manageTab({
    target_id: "target-a",
    document_id: "loader-a",
    action: "activate",
  });

  const after = await browser.reconcile(viewport);
  assert.equal(after.focused_target_id, "target-a");
  assert.equal(
    connection.calls.some(
      (call) => call.method === "Page.bringToFront" && call.sessionId === "session-a",
    ),
    true,
    "activation must use the selected page session so a subsequent focus read sees it",
  );
});

test("tab close settles target removal before reporting completion", async () => {
  const connection = new DelayedTargetCloseConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);

  const closed = await browser.manageTab({
    target_id: "target-b",
    document_id: "loader-b",
    action: "close",
  });

  assert.equal(closed.action, "close");
  assert.equal(connection.closurePolls, 2);
  const reconciled = await browser.reconcile(viewport);
  assert.deepEqual(reconciled.tabs.map((tab) => tab.target_id), ["target-a"]);
  assert.equal(reconciled.focused_target_id, "target-a");
});

test("tab close fails within the request timeout when Chrome retains the target", async () => {
  const connection = new DelayedTargetCloseConnection(Number.POSITIVE_INFINITY);
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
    requestTimeoutMs: 1,
  });
  await browser.reconcile(viewport);

  await assert.rejects(
    browser.manageTab({
      target_id: "target-b",
      document_id: "loader-b",
      action: "close",
    }),
    (error) => error.code === "browser_tab_close_failed" && /within 1ms/.test(error.message),
  );
});

test("legacy navigation and waits reuse the reconciled target session", async () => {
  const connection = new CompatibilityConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);

  const navigation = await browser.navigate({
    target_id: "target-a",
    document_id: "loader-a",
    url: "https://example.test/settings",
  });
  const selector = await browser.wait({
    target_id: "target-a",
    document_id: "loader-next",
    kind: "selector",
    selector: "#settings",
    timeout_ms: 500,
  });
  const idle = await browser.wait({
    target_id: "target-a",
    document_id: "loader-next",
    kind: "idle",
    timeout_ms: 500,
  });

  assert.equal(navigation.document_id, "loader-next");
  assert.equal(selector.kind, "selector");
  assert.equal(idle.kind, "idle");
  assert.equal(
    connection.calls.filter(
      (call) => call.method === "Target.attachToTarget" && call.params.targetId === "target-a",
    ).length,
    1,
    "compatibility calls must retain the controller-owned target session",
  );
});

test("structured snapshots bind compact accessibility and DOM nodes to one document", async () => {
  const connection = new SnapshotConnection();
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
  });
  await browser.reconcile(viewport);

  const first = await browser.snapshot({
    target_id: "target-a",
    document_id: "loader-a",
  });
  const second = await browser.snapshot({
    target_id: "target-a",
    document_id: "loader-a",
  });

  assert.equal(first.browser_generation, 1);
  assert.equal(first.target_id, "target-a");
  assert.equal(first.document_id, "loader-a");
  assert.equal(first.snapshot_revision, 1);
  assert.equal(second.snapshot_revision, 2);
  assert.deepEqual(first.accessibility_nodes, [
    {
      node_ref: "backend:103",
      parent_ref: null,
      child_refs: ["backend:104"],
      role: "button",
      name: "Save",
      description: "Save this form",
      value: "",
      ignored: false,
      disabled: false,
      focused: true,
      states: [],
    },
    {
      node_ref: "backend:104",
      parent_ref: "backend:103",
      child_refs: [],
      role: "StaticText",
      name: "Save",
      description: "",
      value: "",
      ignored: false,
      disabled: false,
      focused: false,
      states: [],
    },
  ]);
  assert.deepEqual(
    first.dom_nodes.find((node) => node.node_ref === "backend:103"),
    {
      node_ref: "backend:103",
      parent_ref: "backend:102",
      document_index: 0,
      node_type: 1,
      node_name: "BUTTON",
      text: "",
      attributes: { id: "save", type: "submit" },
      bounds: { x: 10, y: 20, width: 100, height: 30 },
    },
  );
  assert.equal(
    first.dom_nodes.find((node) => node.node_ref === "backend:200").document_index,
    1,
    "frame nodes must retain the document that owns them",
  );
  assert.deepEqual(
    first.dom_nodes.find((node) => node.node_ref === "backend:105").attributes,
    { type: "password", value: "[redacted]" },
    "password values must never leave the controller snapshot",
  );
  assert.deepEqual(first.dom_documents, [
    { document_index: 0, url: "https://a.test/", owner_node_ref: null },
    { document_index: 1, url: "https://frame.test/", owner_node_ref: "backend:103" },
  ]);
  assert.deepEqual(first.shadow_roots, [
    { node_ref: "backend:102", shadow_root_type: "open" },
  ]);

  connection.loaderId = "loader-a2";
  await assert.rejects(
    browser.snapshot({ target_id: "target-a", document_id: "loader-a" }),
    (error) => error.code === "stale_document_reference",
  );
  connection.loaderId = "loader-a";
  const afterRejectedCapture = await browser.snapshot({
    target_id: "target-a",
    document_id: "loader-a",
  });
  assert.equal(
    afterRejectedCapture.snapshot_revision,
    3,
    "a rejected capture must not consume a snapshot revision",
  );
});

test("structured snapshot strings are bounded by UTF-8 bytes", async () => {
  const connection = new SnapshotConnection();
  connection.accessibilityName = "😀".repeat(2_000);
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
  });
  await browser.reconcile(viewport);

  const snapshot = await browser.snapshot({
    target_id: "target-a",
    document_id: "loader-a",
  });

  assert.equal(
    new TextEncoder().encode(snapshot.accessibility_nodes[0].name).length,
    2_048,
  );
});

test("dialog handling is document-bound and validates prompt input", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
  });
  await browser.reconcile(viewport);
  const frameTreeCallsBeforeDialog = connection.calls.filter(
    (call) => call.method === "Page.getFrameTree",
  ).length;

  const accepted = await browser.handleDialog({
    target_id: "target-a",
    document_id: "loader-a",
    action: "accept",
    prompt_text: "answer",
  });

  assert.deepEqual(accepted, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    action: "accept",
  });
  assert.deepEqual(
    connection.calls.find((call) => call.method === "Page.handleJavaScriptDialog").params,
    { accept: true, promptText: "answer" },
  );
  await browser.handleDialog({
    target_id: "target-a",
    document_id: "loader-a",
    action: "accept",
    prompt_text: "",
  });
  assert.deepEqual(
    connection.calls.filter((call) => call.method === "Page.handleJavaScriptDialog").at(-1).params,
    { accept: true, promptText: "" },
  );
  await browser.handleDialog({
    target_id: "target-a",
    document_id: "loader-a",
    action: "dismiss",
    prompt_text: null,
  });
  assert.deepEqual(
    connection.calls.filter((call) => call.method === "Page.handleJavaScriptDialog").at(-1).params,
    { accept: false },
  );
  assert.equal(
    connection.calls.filter((call) => call.method === "Page.getFrameTree").length,
    frameTreeCallsBeforeDialog,
    "dialog handling must use the last controller-observed document because page commands block while a dialog is open",
  );
  await assert.rejects(
    browser.handleDialog({
      target_id: "target-a",
      document_id: "loader-b",
      action: "dismiss",
    }),
    (error) => error.code === "stale_document_reference",
  );
  await assert.rejects(
    browser.handleDialog({
      target_id: "target-a",
      document_id: "loader-a",
      action: "ignore",
    }),
    (error) => error.code === "browser_dialog_invalid",
  );
});

test("download and upload requests stay target-bound and return no file paths", async () => {
  const connection = new FakeConnection();
  const entries = new Map([
    ["/safe/downloads", { type: "directory", size: 0 }],
    ["/safe/uploads", { type: "directory", size: 0 }],
    ["/safe/uploads/report.txt", { type: "file", size: 12 }],
  ]);
  const fileSystem = {
    mkdir: async () => {},
    realpath: async (candidate) => {
      if (!entries.has(candidate)) throw Object.assign(new Error("missing"), { code: "ENOENT" });
      return candidate;
    },
    stat: async (candidate) => {
      const entry = entries.get(candidate);
      if (!entry) throw Object.assign(new Error("missing"), { code: "ENOENT" });
      return {
        size: entry.size,
        isDirectory: () => entry.type === "directory",
        isFile: () => entry.type === "file",
      };
    },
    statfs: async () => ({ bavail: 1024 * 1024 * 1024, bsize: 1 }),
  };
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
    downloadDirectory: "/safe/downloads",
    uploadRoots: ["/safe/uploads"],
    fileSystem,
    stageUploads: async ({ files }) => ({ files, markExposed() {}, async discard() {} }),
  });
  await browser.reconcile(viewport);

  const downloads = await browser.configureDownloads({
    target_id: "target-a",
    document_id: "loader-a",
  });
  const upload = await browser.uploadFiles({
    target_id: "target-a",
    document_id: "loader-a",
    node_ref: "backend:103",
    file_paths: ["/safe/uploads/report.txt"],
  });

  assert.deepEqual(downloads, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    enabled: true,
  });
  assert.deepEqual(upload, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    file_count: 1,
    total_bytes: 12,
  });
  assert.equal(JSON.stringify({ downloads, upload }).includes("/safe"), false);
});

test("prompt defaults remain document-bound, private, and cleared when the dialog lifecycle ends", async () => {
  let connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  const target = { target_id: "target-a", document_id: "loader-a", action: "accept" };
  await browser.reconcile(viewport);
  const open = () => connection.emit({
    method: "Page.javascriptDialogOpening", sessionId: "session-a",
    params: { type: "prompt", defaultPrompt: "private default", message: "private message" },
  });
  const lastAnswer = () => connection.calls.filter((call) => call.method === "Page.handleJavaScriptDialog").at(-1).params;
  open();
  const result = await browser.handleDialog(target);
  assert.deepEqual(lastAnswer(), { accept: true, promptText: "private default" });
  assert.equal(JSON.stringify(result).includes("private"), false);
  const trace = browser.pollEvents({ cursor: 0, browser_generation: 1 });
  assert.equal(JSON.stringify(trace).includes("private"), false);
  await assert.rejects(browser.handleDialog({ ...target, document_id: "old-document" }), (error) => error.code === "stale_document_reference");
  await browser.handleDialog({ ...target, prompt_text: "" });
  assert.deepEqual(lastAnswer(), { accept: true, promptText: "" });
  await browser.handleDialog({ target_id: "target-b", document_id: "loader-b", action: "accept" });
  assert.deepEqual(lastAnswer(), { accept: true });

  for (const event of [
    { method: "Page.javascriptDialogClosed", sessionId: "session-a", params: { result: true } },
    { method: "Page.frameNavigated", sessionId: "session-a", params: { frame: { id: "frame-a", loaderId: "loader-a" } } },
    { method: "Target.targetCrashed", params: { targetId: "target-a" } },
    { method: "Target.targetDestroyed", params: { targetId: "target-a" } },
    { method: "Inspector.targetCrashed", sessionId: "session-a", params: {} },
    { method: "Target.detachedFromTarget", params: { sessionId: "session-a", targetId: "target-a" } },
  ]) {
    open();
    connection.emit(event);
    await browser.handleDialog(target);
    assert.deepEqual(lastAnswer(), { accept: true }, event.method);
  }
  open();
  await browser.close();
  connection = new FakeConnection();
  await browser.reconcile(viewport);
  await browser.handleDialog(target);
  assert.deepEqual(lastAnswer(), { accept: true });
  await browser.close();
});

test("permission decisions derive the current target origin", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);

  const result = await browser.setPermission({
    target_id: "target-a",
    document_id: "loader-a",
    permission: "geolocation",
    setting: "prompt",
  });

  assert.deepEqual(result, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    permission: "geolocation",
    setting: "prompt",
  });
  assert.deepEqual(
    connection.calls.find((call) => call.method === "Browser.setPermission").params,
    {
      permission: { name: "geolocation" },
      setting: "prompt",
      origin: "https://a.test",
    },
  );
});

test("controller event polling uses the reconciliation cursor and persistent session mapping", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  const reconciled = await browser.reconcile(viewport);
  connection.emit({
    method: "Runtime.consoleAPICalled",
    sessionId: "session-a",
    params: { type: "warning", args: [{ value: "must-not-leave-controller" }] },
  });

  const replay = browser.pollEvents({
    browser_generation: reconciled.browser_generation,
    cursor: reconciled.event_cursor,
    limit: 10,
  });
  assert.equal(replay.replay_gap, false);
  assert.equal(replay.events.length, 1);
  assert.equal(replay.events[0].target_id, "target-a");
  assert.equal(replay.events[0].document_id, "loader-a");
  assert.equal(JSON.stringify(replay).includes("must-not-leave-controller"), false);

  connection.emit({
    method: "Browser.downloadWillBegin",
    params: { frameId: "frame-a", guid: "download-a", url: "https://a.test/file?secret=1", suggestedFilename: "report.txt" },
  });
  connection.emit({
    method: "Browser.downloadProgress",
    params: { guid: "download-a", state: "completed", receivedBytes: 12, totalBytes: 12 },
  });
  const downloads = browser.pollEvents({
    browser_generation: reconciled.browser_generation,
    cursor: replay.next_cursor,
    limit: 10,
  });
  assert.deepEqual(downloads.events.map((event) => event.target_id), ["target-a", "target-a"]);
  assert.equal(JSON.stringify(downloads).includes("secret"), false);
});

test("controller cancels every active download when slice storage drops below its reserve", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
    downloadDirectory: "/safe/downloads",
    minimumDownloadFreeBytes: 256 * 1024 * 1024,
    fileSystem: {
      statfs: async () => ({ bavail: 128 * 1024 * 1024, bsize: 1 }),
    },
  });
  const reconciled = await browser.reconcile(viewport);

  for (const [frameId, guid] of [["frame-a", "download-a"], ["frame-b", "download-b"]]) {
    connection.emit({
      method: "Browser.downloadWillBegin",
      params: { frameId, guid, url: "https://example.test/file", suggestedFilename: "file.bin" },
    });
  }
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    connection.calls
      .filter((call) => call.method === "Browser.cancelDownload")
      .map((call) => call.params.guid)
      .sort(),
    ["download-a", "download-b"],
  );
  connection.emit({
    method: "Browser.downloadProgress",
    params: { guid: "download-a", state: "canceled", receivedBytes: 1, totalBytes: 10 },
  });
  const events = browser.pollEvents({
    browser_generation: reconciled.browser_generation,
    cursor: reconciled.event_cursor,
    limit: 10,
  }).events;
  const canceled = events.find((event) => event.kind === "download_progress");
  assert.equal(canceled.data.cancellation_reason, "disk_pressure");
});

test("controller guards downloads before their frame is mapped to a tab", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
    downloadDirectory: "/safe/downloads",
    minimumDownloadFreeBytes: 256 * 1024 * 1024,
    fileSystem: {
      statfs: async () => ({ bavail: 128 * 1024 * 1024, bsize: 1 }),
    },
  });
  const reconciled = await browser.reconcile(viewport);

  connection.emit({
    method: "Browser.downloadWillBegin",
    params: {
      frameId: "frame-not-yet-observed",
      guid: "download-before-frame-map",
      url: "https://example.test/immediate-download",
      suggestedFilename: "immediate.bin",
    },
  });
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    connection.calls
      .filter((call) => call.method === "Browser.cancelDownload")
      .map((call) => call.params.guid),
    ["download-before-frame-map"],
  );
  const started = browser.pollEvents({
    browser_generation: reconciled.browser_generation,
    cursor: reconciled.event_cursor,
    limit: 10,
  }).events.find((event) => event.kind === "download_started");
  assert.equal(started.target_id, null, "optional attribution must not gate disk safety");
});

test("a download arriving during disk-pressure cancellation receives a follow-up check", async () => {
  const connection = new FakeConnection();
  const releaseFirstCancellation = Promise.withResolvers();
  const send = connection.send.bind(connection);
  connection.send = async (method, params = {}, sessionId) => {
    const result = await send(method, params, sessionId);
    if (method === "Browser.cancelDownload" && params.guid === "download-a") {
      await releaseFirstCancellation.promise;
    }
    return result;
  };
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
    downloadDirectory: "/safe/downloads",
    minimumDownloadFreeBytes: 256 * 1024 * 1024,
    fileSystem: {
      statfs: async () => ({ bavail: 128 * 1024 * 1024, bsize: 1 }),
    },
  });
  await browser.reconcile(viewport);

  connection.emit({
    method: "Browser.downloadWillBegin",
    params: {
      frameId: "frame-a",
      guid: "download-a",
      url: "https://example.test/a",
      suggestedFilename: "a.bin",
    },
  });
  while (!connection.calls.some(
    (call) => call.method === "Browser.cancelDownload" && call.params.guid === "download-a",
  )) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  connection.emit({
    method: "Browser.downloadWillBegin",
    params: {
      frameId: "frame-b",
      guid: "download-b",
      url: "https://example.test/b",
      suggestedFilename: "b.bin",
    },
  });
  releaseFirstCancellation.resolve();
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    connection.calls
      .filter((call) => call.method === "Browser.cancelDownload")
      .map((call) => call.params.guid),
    ["download-a", "download-b"],
  );
});

test("debugger discovery cannot redirect the controller away from loopback", () => {
  assert.equal(
    assertPrivateDebuggerUrl(
      "ws://localhost:9222/devtools/browser/one",
      "http://127.0.0.1:9222",
    ),
    "ws://localhost:9222/devtools/browser/one",
  );
  assert.throws(
    () =>
      assertPrivateDebuggerUrl(
        "ws://example.test:9222/devtools/browser/one",
        "http://127.0.0.1:9222",
      ),
    (error) => error.code === "browser_debugger_endpoint_unsafe",
  );
  assert.throws(
    () =>
      assertPrivateDebuggerUrl(
        "ws://127.0.0.1:9333/devtools/browser/one",
        "http://127.0.0.1:9222",
      ),
    (error) => error.code === "browser_debugger_endpoint_unsafe",
  );
});

test("CDP responses are correlated and command failures stay bounded", async () => {
  const socket = new FakeSocket();
  const connection = new CdpConnection(socket, 20);
  const response = connection.send("Target.getTargets");
  const request = JSON.parse(socket.sent[0]);
  socket.message({ id: request.id, result: { targetInfos: [] } });
  assert.deepEqual(await response, { targetInfos: [] });

  const dialog = connection.waitForEvent("Page.javascriptDialogOpening", 20);
  const observed = [];
  connection.subscribe((event) => observed.push(event.method));
  socket.message({
    method: "Page.javascriptDialogOpening",
    sessionId: "session-a",
    params: { type: "prompt", message: "Name?" },
  });
  assert.deepEqual(await dialog.promise, {
    method: "Page.javascriptDialogOpening",
    sessionId: "session-a",
    params: { type: "prompt", message: "Name?" },
  });
  assert.deepEqual(observed, ["Page.javascriptDialogOpening"]);

  const timedOut = connection.send("Page.getFrameTree", {}, "session-a");
  await assert.rejects(timedOut, (error) => error.code === "browser_cdp_timeout");
  await connection.close();
});

test("timed-out or disconnected dialog replies terminate and do not leak defaults into a new connection", async (t) => {
  let socket = new DialogFaultSocket();
  const browser = new BrowserCdpClient({ connectionFactory: async () => new CdpConnection(socket, 100) });
  t.after(() => browser.close());
  let id = 0;
  const request = (method, params) => handleBrowserControllerRequest(
    { id: ++id, method, params },
    {
      browser,
      resourceInventory: async () => ({
        browser_ids: ["browser-pid-fixture"],
        profile_ids: ["profile-sha256-fixture"],
      }),
    },
  );
  const target = { target_id: "target-a", document_id: "loader-a", action: "accept" };
  assert.equal((await request("browser.reconcile", { viewport })).ok, true);
  socket.message({ method: "Page.javascriptDialogOpening", sessionId: "session-a", params: { type: "prompt", defaultPrompt: "old private default" } });
  socket.holdDialog = true;
  const timedOut = await request("browser.dialog", target);
  assert.equal(timedOut.ok, false);
  assert.equal(timedOut.error.code, "browser_cdp_timeout");
  // A late acknowledgement belongs only to the expired request.
  socket.message({ id: socket.heldDialog.id, result: {} });
  socket.holdDialog = false;
  assert.equal((await request("browser.dialog", target)).ok, true);
  assert.equal(socket.lastDialog.params.promptText, "old private default");

  socket.holdDialog = true;
  socket.dialogSent = Promise.withResolvers();
  const disconnectedRequest = request("browser.dialog", target);
  await socket.dialogSent.promise;
  socket.close();
  const disconnected = await disconnectedRequest;
  assert.equal(disconnected.ok, false);
  assert.equal(disconnected.error.code, "browser_cdp_disconnected");
  socket = new DialogFaultSocket();
  const reconnected = await request("browser.reconcile", { viewport });
  assert.equal(reconnected.ok, true, JSON.stringify(reconnected.error));
  assert.equal(reconnected.result.browser_generation, 2);
  assert.equal((await request("browser.dialog", target)).ok, true);
  assert.deepEqual(socket.lastDialog.params, { accept: true }, "a new connection must not reuse the previous dialog default");
  const trace = await request("browser.events.poll", { cursor: 0, browser_generation: 2 });
  assert.equal(JSON.stringify(trace).includes("old private default"), false);
});

test("any command's new controller connection sweeps leftover App Tabs", async () => {
  const connection = new FakeConnection();
  connection.extraTargets = [{ targetId: "app-left", type: "page", url: "https://a1.app.chariox.internal/", title: "App" }];
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  const response = await handleBrowserControllerRequest({ id: 1, method: "browser.reconcile", params: { viewport } }, {
    browser, resourceInventory: async () => ({ browser_ids: ["browser-pid-fixture"], profile_ids: ["profile-sha256-fixture"] }),
  });
  assert.equal(response.ok, true, JSON.stringify(response.error));
  assert.ok(connection.closedTargetIds.has("app-left"), "the non-App command's connection ran the App sweep");
  assert.ok(!connection.closedTargetIds.has("target-a"));
  // The sweep finishes before the command lists Tabs: a restored App window
  // is never inspected while it closes (a slice restart's first reconcile).
  assert.deepEqual(response.result.tabs.map((tab) => tab.target_id), ["target-a", "target-b"]);
  assert.ok(!connection.calls.some((call) => call.method === "Target.attachToTarget" && call.params.targetId === "app-left"));
});

test("a command on a session that detaches fails at once, not at its timeout", async () => {
  const socket = new FakeSocket();
  const connection = new CdpConnection(socket, 60_000);
  const closing = connection.send("Page.enable", {}, "session-a");
  const other = connection.send("Page.enable", {}, "session-b");
  socket.message({ method: "Target.detachedFromTarget", params: { sessionId: "session-a", targetId: "target-a" } });
  await assert.rejects(closing, (error) => error.code === "browser_cdp_command_failed" && /Page\.enable: Target closed/.test(error.message));
  socket.message({ id: JSON.parse(socket.sent[1]).id, result: {} });
  assert.deepEqual(await other, {});
  await connection.close();
});

test("a Tab that closes while its session starts drops out of the reconcile at once", async () => {
  // Chromium does not answer the commands of a session whose target closed.
  const socket = new ClosingTabSocket("target-a", "session-a");
  const browser = new BrowserCdpClient({ connectionFactory: async () => new CdpConnection(socket, 60_000) });
  const reconciled = await browser.reconcile(viewport);
  assert.deepEqual(reconciled.tabs.map((tab) => tab.target_id), ["target-b"]);
  await browser.close();
});

test("a Tab still listed after its session went away twice is left for the next reconcile", async () => {
  const connection = new StillClosingConnection("session-a");
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  const reconciled = await browser.reconcile(viewport);
  assert.deepEqual(reconciled.tabs.map((tab) => tab.target_id), ["target-b"]);
  connection.closingSessionId = null;
  assert.deepEqual((await browser.reconcile(viewport)).tabs.map((tab) => tab.target_id), ["target-a", "target-b"]);
});

class FakeConnection {
  constructor() {
    this.calls = [];
    this.open = true;
    this.listeners = new Set();
    this.includeWorker = true;
    this.closedTargetIds = new Set();
  }

  isOpen() {
    return this.open;
  }

  async close() {
    this.open = false;
  }

  subscribe(listener) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  emit(message) {
    for (const listener of this.listeners) listener(message);
  }

  async send(method, params = {}, sessionId) {
    this.calls.push({ method, params, sessionId });
    if (method === "Emulation.setDeviceMetricsOverride" && this.vanishOnInspect?.[sessionId]) {
      const targetId = this.vanishOnInspect[sessionId];
      delete this.vanishOnInspect[sessionId];
      this.closedTargetIds.add(targetId);
      throw new Error(`Emulation.setDeviceMetricsOverride: Session with given id not found.`);
    }
    if (method === 'Runtime.evaluate' && params.expression === '({ width: innerWidth, height: innerHeight })') return { result: { value: { width: 3840, height: 2160 } } };
    if (method === 'Browser.getWindowForTarget') return { windowId: 1, bounds: { left: 0, top: 0, width: 3840, height: 2160 } };
    if (method === "Target.setDiscoverTargets" && this.failDiscovery) {
      throw new Error("discovery failed");
    }
    if (method === "Target.getTargets") {
      return {
        targetInfos: [
          { targetId: "target-a", type: "page", url: "https://a.test/", title: "A" },
          ...(this.includeWorker
            ? [{ targetId: "worker-a", type: "worker", url: "https://a.test/worker.js" }]
            : []),
          { targetId: "target-b", type: "page", url: "https://b.test/", title: "B" },
          ...(this.extraTargets ?? []),
        ].filter(({ targetId }) => !this.closedTargetIds.has(targetId)),
      };
    }
    if (method === "Target.attachToTarget") {
      const sessions = {
        "target-a": "session-a",
        "target-b": "session-b",
        "worker-a": "worker-session-a",
        ...Object.fromEntries((this.extraTargets ?? []).map(({ targetId }) => [targetId, `session-${targetId}`])),
      };
      return { sessionId: sessions[params.targetId] };
    }
    if (method === "Target.closeTarget") {
      this.closedTargetIds.add(params.targetId);
      return { success: true };
    }
    if (method === "Page.getFrameTree") {
      return {
        frameTree: {
          frame: {
            id: sessionId === "session-a" ? "frame-a" : "frame-b",
            loaderId: sessionId === "session-a" ? "loader-a" : "loader-b",
          },
        },
      };
    }
    if (method === "Page.createIsolatedWorld") {
      this.isolatedWorlds = (this.isolatedWorlds ?? 0) + 1;
      return { executionContextId: 100 + this.isolatedWorlds };
    }
    if (method === "Runtime.evaluate") {
      if (sessionId === "worker-session-a" && params.expression === "self.close()") {
        this.closedTargetIds.add("worker-a");
      }
      return { result: { value: sessionId === "session-b" } };
    }
    if (method === "DOM.resolveNode") return { object: { objectId: "file-object" } };
    if (method === "Runtime.callFunctionOn") return { result: { value: "file" } };
    return {};
  }
}

class StillClosingConnection extends FakeConnection {
  constructor(closingSessionId) {
    super();
    this.closingSessionId = closingSessionId;
  }

  async send(method, params = {}, sessionId) {
    if (method === "Emulation.setDeviceMetricsOverride" && sessionId === this.closingSessionId) {
      this.calls.push({ method, params, sessionId });
      throw new Error(`${method}: Session with given id not found.`);
    }
    return super.send(method, params, sessionId);
  }
}

class PageActivationConnection extends FakeConnection {
  constructor() {
    super();
    this.focusedTargetId = "target-b";
  }

  async send(method, params = {}, sessionId) {
    if (method === "Page.bringToFront") {
      this.focusedTargetId = sessionId === "session-a" ? "target-a" : "target-b";
      return super.send(method, params, sessionId);
    }
    if (method === "Runtime.evaluate" && params.expression === "document.visibilityState === 'visible'") {
      this.calls.push({ method, params, sessionId });
      return {
        result: {
          value: this.focusedTargetId === (sessionId === "session-a" ? "target-a" : "target-b"),
        },
      };
    }
    return super.send(method, params, sessionId);
  }
}

class ReleaseFaultConnection extends FakeConnection {
  constructor() {
    super();
    this.failRelease = true;
  }

  async send(method, params = {}, sessionId) {
    if (this.failRelease && method === "Page.setWebLifecycleState" && params.state === "active") {
      this.calls.push({ method, params, sessionId });
      throw new Error("fixture release failure");
    }
    return super.send(method, params, sessionId);
  }
}

class AcquisitionFaultConnection extends FakeConnection {
  constructor() {
    super();
    this.failAcquisition = true;
  }

  async send(method, params = {}, sessionId) {
    if (this.failAcquisition && method === "Page.stopLoading") {
      this.calls.push({ method, params, sessionId });
      throw new Error("fixture acquisition failure");
    }
    return super.send(method, params, sessionId);
  }
}

class DelayedTargetCloseConnection extends FakeConnection {
  constructor(closurePollsBeforeRemoval = 2) {
    super();
    this.closingTargetId = null;
    this.closurePolls = 0;
    this.closurePollsBeforeRemoval = closurePollsBeforeRemoval;
  }

  async send(method, params = {}, sessionId) {
    if (method === "Target.closeTarget") {
      this.calls.push({ method, params, sessionId });
      this.closingTargetId = params.targetId;
      return { success: true };
    }
    if (method === "Target.getTargets" && this.closingTargetId) {
      this.calls.push({ method, params, sessionId });
      this.closurePolls += 1;
      const targetInfos = [
        { targetId: "target-a", type: "page", url: "https://a.test/", title: "A" },
        { targetId: "target-b", type: "page", url: "https://b.test/", title: "B" },
      ].filter((target) => (
        target.targetId !== this.closingTargetId
        || this.closurePolls < this.closurePollsBeforeRemoval
      ));
      return { targetInfos };
    }
    if (method === "Runtime.evaluate" && sessionId === "session-a" && this.closurePolls >= 2) {
      this.calls.push({ method, params, sessionId });
      return { result: { value: true } };
    }
    return super.send(method, params, sessionId);
  }
}

class SnapshotConnection extends FakeConnection {
  constructor() {
    super();
    this.loaderId = "loader-a";
    this.accessibilityName = "Save";
  }

  async send(method, params = {}, sessionId) {
    if (method === "Page.getFrameTree") {
      this.calls.push({ method, params, sessionId });
      return { frameTree: { frame: { loaderId: this.loaderId } } };
    }
    if (method === "Accessibility.getFullAXTree") {
      this.calls.push({ method, params, sessionId });
      return {
        nodes: [
          {
            nodeId: "ax-button",
            backendDOMNodeId: 103,
            ignored: false,
            role: { value: "button" },
            name: { value: this.accessibilityName },
            description: { value: "Save this form" },
            childIds: ["ax-text"],
            properties: [
              { name: "focused", value: { value: true } },
              { name: "disabled", value: { value: false } },
            ],
          },
          {
            nodeId: "ax-text",
            backendDOMNodeId: 104,
            parentId: "ax-button",
            ignored: false,
            role: { value: "StaticText" },
            name: { value: "Save" },
          },
        ],
      };
    }
    if (method === "DOMSnapshot.captureSnapshot") {
      this.calls.push({ method, params, sessionId });
      return {
        strings: [
          "#document",
          "HTML",
          "BODY",
          "BUTTON",
          "#text",
          "Save",
          "id",
          "save",
          "type",
          "submit",
          "INPUT",
          "password",
          "value",
          "top-secret",
          "https://a.test/",
          "https://frame.test/",
          "open",
        ],
        documents: [
          {
            nodes: {
              parentIndex: [-1, 0, 1, 2, 3, 2],
              nodeType: [9, 1, 1, 1, 3, 1],
              nodeName: [0, 1, 2, 3, 4, 10],
              nodeValue: [0, 0, 0, 0, 5, 0],
              backendNodeId: [100, 101, 102, 103, 104, 105],
              attributes: [[], [], [], [6, 7, 8, 9], [], [8, 11, 12, 13]],
              contentDocumentIndex: { index: [3], value: [1] },
              shadowRootType: { index: [2], value: [16] },
            },
            documentURL: 14,
            layout: {
              nodeIndex: [3],
              bounds: [[10, 20, 100, 30]],
            },
          },
          {
            nodes: {
              parentIndex: [-1],
              nodeType: [9],
              nodeName: [0],
              nodeValue: [0],
              backendNodeId: [200],
              attributes: [[]],
            },
            documentURL: 15,
            layout: { nodeIndex: [], bounds: [] },
          },
        ],
      };
    }
    return super.send(method, params, sessionId);
  }
}

class CompatibilityConnection extends FakeConnection {
  constructor() {
    super();
    this.loaderId = "loader-a";
  }

  async send(method, params = {}, sessionId) {
    if (method === "Page.getFrameTree" && sessionId === "session-a") {
      this.calls.push({ method, params, sessionId });
      return { frameTree: { frame: { id: "frame-a", loaderId: this.loaderId } } };
    }
    if (method === "Page.navigate") {
      this.calls.push({ method, params, sessionId });
      this.loaderId = "loader-next";
      return { loaderId: this.loaderId };
    }
    if (
      method === "Runtime.evaluate" &&
      (params.expression.includes("querySelector") || params.expression.includes("readyState"))
    ) {
      this.calls.push({ method, params, sessionId });
      return { result: { value: true } };
    }
    return super.send(method, params, sessionId);
  }
}

class FakeSocket extends EventTarget {
  constructor() {
    super();
    this.readyState = 1;
    this.sent = [];
  }

  send(payload) {
    this.sent.push(payload);
  }

  close() {
    this.readyState = 3;
    this.dispatchEvent(new Event("close"));
  }

  message(payload) {
    this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) }));
  }
}

class ClosingTabSocket extends FakeSocket {
  constructor(targetId, sessionId) {
    super();
    this.remote = new FakeConnection();
    this.targetId = targetId;
    this.sessionId = sessionId;
  }

  send(payload) {
    super.send(payload);
    const request = JSON.parse(payload);
    if (request.method === "Page.enable" && request.sessionId === this.sessionId) {
      this.remote.closedTargetIds.add(this.targetId);
      this.message({ method: "Target.detachedFromTarget", params: { sessionId: this.sessionId, targetId: this.targetId } });
      return;
    }
    void this.remote.send(request.method, request.params, request.sessionId)
      .then((result) => this.message({ id: request.id, result }));
  }
}

class DialogFaultSocket extends FakeSocket {
  constructor() {
    super();
    this.remote = new FakeConnection();
    this.holdDialog = false;
    this.dialogSent = Promise.withResolvers();
  }

  send(payload) {
    super.send(payload);
    const request = JSON.parse(payload);
    if (request.method === "Page.handleJavaScriptDialog") {
      this.lastDialog = request;
      this.dialogSent.resolve();
      if (this.holdDialog) {
        this.heldDialog = request;
        return;
      }
    }
    void this.remote.send(request.method, request.params, request.sessionId)
      .then((result) => this.message({ id: request.id, result }));
  }
}

 test("canonical display refusal prevents CDP layout changes", async () => {
  let connects = 0;
  const browser = new BrowserCdpClient({
    applyCanonicalDisplay: async () => { throw new Error("physical display refused"); },
    connectionFactory: async () => { connects += 1; return new FakeConnection(); },
  });
  await assert.rejects(browser.reconcile(viewport), /physical display refused/);
  assert.equal(connects, 0);
  assert.equal(browser.appViewport, undefined);
});

test("closing the final page creates a blank page before closing the window", async () => {
  const browser = new BrowserCdpClient();
  const calls = [];
  const connection = { async send(method, params) {
    calls.push([method, params]);
    if (method === "Target.getTargets") return { targetInfos: [{type:"page",targetId:"last"}] };
    return {success:true,targetId:"blank"};
  }};
  await browser.closePageTarget(connection, "last");
  assert.deepEqual(calls, [
    ["Target.getTargets", undefined],
    ["Target.createTarget", {url:"about:blank"}],
    ["Target.closeTarget", {targetId:"last"}],
  ]);
});

test("failed replacement leaves the last page open and releases the close queue", async () => {
  const browser = new BrowserCdpClient();
  let failure = true;
  let closed = 0;
  const connection = { async send(method) {
    if (method === "Target.getTargets") return {targetInfos:[{type:"page",targetId:"last"}]};
    if (method === "Target.createTarget" && failure) throw new Error("cannot create page");
    if (method === "Target.closeTarget") closed++;
    return {success:true};
  }};
  await assert.rejects(browser.closePageTarget(connection, "last"), /cannot create page/);
  assert.equal(closed, 0);
  failure = false;
  await browser.closePageTarget(connection, "last");
  assert.equal(closed, 1);
});

test("concurrent closes preserve a page after the final original tab closes", async () => {
  const browser = new BrowserCdpClient();
  const pages = new Set(["a", "b"]);
  const connection = { async send(method, params) {
    await new Promise(resolve => setImmediate(resolve));
    if (method === "Target.getTargets") return {targetInfos:[...pages].map(targetId=>({type:"page",targetId}))};
    if (method === "Target.createTarget") pages.add("blank");
    if (method === "Target.closeTarget") pages.delete(params.targetId);
    assert.ok(pages.size);
    return {success:true};
  }};
  await Promise.all([browser.closePageTarget(connection,"a"),browser.closePageTarget(connection,"b")]);
  assert.deepEqual([...pages], ["blank"]);
});

// MP-08/MP-10: a partially failed resize is never an observational preflight.
test("failed viewport change invalidates concurrent reconciliation admission", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);
  assert.equal(browser.canReconcileConcurrently(viewport), true);
  const send = connection.send.bind(connection);
  connection.send = async (method, params, sessionId) => {
    if (method === "Emulation.setDeviceMetricsOverride") throw new Error("resize failed");
    return send(method, params, sessionId);
  };
  await assert.rejects(browser.reconcile({ ...viewport, css_width: 900 }), /resize failed/);
  assert.equal(browser.canReconcileConcurrently(viewport), false);
  await browser.close();
});

// The Room browser bar and App panel layout are applied with the viewport: a
// change to either is never an observational preflight.
test("browser bar and App panel changes are not admitted as concurrent reconciliation", async () => {
  const browser = new BrowserCdpClient({ connectionFactory: async () => new FakeConnection() });
  const layout = { browserBarVisible: false, appPanelCssWidth: 320 };
  await browser.reconcile(viewport, layout);
  assert.equal(browser.canReconcileConcurrently(viewport, layout), true);
  assert.equal(browser.canReconcileConcurrently(viewport, { ...layout, browserBarVisible: true }), false);
  assert.equal(browser.canReconcileConcurrently(viewport, { ...layout, appPanelCssWidth: 400 }), false);
  assert.equal(browser.canReconcileConcurrently(viewport), false);
  await browser.close();
});

test("a failed physical display apply withdraws concurrent reconciliation admission", async () => {
  let refuse = false;
  const browser = new BrowserCdpClient({
    applyCanonicalDisplay: async () => { if (refuse) throw new Error("physical display refused"); },
    connectionFactory: async () => new FakeConnection(),
  });
  await browser.reconcile(viewport);
  assert.equal(browser.canReconcileConcurrently(viewport), true);
  refuse = true;
  await assert.rejects(browser.reconcile(viewport), /physical display refused/);
  assert.equal(browser.canReconcileConcurrently(viewport), false);
  await browser.close();
});

test("isolated-world focus keeps the physical visibility captured by input emulation", async () => {
  const browser = new BrowserCdpClient({ connectionFactory: async () => new FakeConnection() });
  assert.equal((await browser.reconcile(viewport)).focused_target_id, "target-b");
  browser.inputCapture.visibilityBySession.set("session-b", { visible: false });
  assert.equal((await browser.reconcile(viewport)).focused_target_id, null);
  browser.inputCapture.visibilityBySession.delete("session-b");
  assert.equal((await browser.reconcile(viewport)).focused_target_id, "target-b");
  await browser.close();
});

// MP-08/MP-10: a native viewer click can navigate between the frame read and
// the focus poll; the destroyed world is a stale document, which reconcile retries.
test("MP-08/MP-10 a focus poll racing a navigation reports a stale document", async () => {
  const browser = new BrowserCdpClient({ connectionFactory: async () => ({}) });
  const connection = { async send(method) {
    if (method === "Page.createIsolatedWorld") return { executionContextId: 3 };
    throw new Error("Runtime.evaluate: Cannot find context with specified id");
  } };
  await assert.rejects(browser.readFocus(connection, "s", "t", { id: "f", loaderId: "d" }), error => error.code === "stale_document_reference");
  assert.equal(browser.focusWorldsByTarget.has("t"), false);
});

test("MP-08 CDP reconciliation retains Chromium popup creation origins", async () => {
  const connection = new FakeConnection(), send = connection.send.bind(connection)
  connection.send = async (method, ...args) => {
    const result = await send(method, ...args)
    if (method === "Target.getTargets") result.targetInfos.find(target => target.targetId === "target-b").openerId = "target-a"
    return result
  }
  const browser = new BrowserCdpClient({connectionFactory: async () => connection})
  const state = await browser.reconcile(viewport)
  assert.equal(state.tabs.find(tab => tab.target_id === "target-b").opener_target_id, "target-a")
  assert(!Object.hasOwn(state.tabs.find(tab => tab.target_id === "target-a"), "opener_target_id"))
})

// MP-08/MP-10/MP-11: fail-first real BBC seam: snapshot_frame_limit during state.
test('ordinary tab reconciliation survives more than64 unrelated advertising frame targets',async()=>{
 const connection=new FakeConnection();connection.extraTargets=Array.from({length:80},(_,i)=>({targetId:'ad-'+i,type:'iframe'}));
 const browser=new BrowserCdpClient({connectionFactory:async()=>connection});
 try{const state=await browser.reconcile(viewport);assert.equal(state.tabs.length,2);assert.equal(connection.calls.filter(c=>c.method==='Target.attachToTarget'&&c.params.targetId.startsWith('ad-')).length,0,'state must not attach unrelated iframe targets; isolated listeners own bounded child sessions')}finally{await browser.close()}
})


test('MP-08/MP-10/MP-11 cached mirror frames follow navigation and retire with their session', async () => {
 const connection=new FakeConnection(),browser=new BrowserCdpClient({connectionFactory:async()=>connection});
 await browser.ensureConnection();const sessionId=await browser.ensureTargetSession(connection,'target-a');
 browser.recordConnectionEvent({method:'Page.frameNavigated',sessionId,params:{frame:{id:'frame-a',loaderId:'doc-a'}}});
 const cached=browser.currentPageFrame('target-a');assert.equal(cached.frame.loaderId,'doc-a');assert.ok(Number.isFinite(cached.navigatedAt));
 browser.recordConnectionEvent({method:'Page.frameStartedNavigating',sessionId,params:{frameId:'frame-a'}});
 assert.equal(browser.currentPageFrame('target-a'),null);
 browser.recordConnectionEvent({method:'Page.frameNavigated',sessionId,params:{frame:{id:'frame-a',loaderId:'doc-b'}}});
 assert.equal(browser.currentPageFrame('target-a').frame.loaderId,'doc-b');
 browser.recordConnectionEvent({method:'Target.detachedFromTarget',params:{sessionId}});
 assert.equal(browser.currentPageFrame('target-a'),null);await browser.close();
});

test('a same-document navigation keeps the cached page frame of its document', async () => {
 const connection=new FakeConnection(),browser=new BrowserCdpClient({connectionFactory:async()=>connection});
 await browser.ensureConnection();const sessionId=await browser.ensureTargetSession(connection,'target-a');
 const event=(method,params)=>browser.recordConnectionEvent({method,sessionId,params});
 event('Page.frameNavigated',{frame:{id:'frame-a',loaderId:'doc-a'}});
 const cached=browser.currentPageFrame('target-a');
 // Fragment links and pushState load but do not navigate: the record is never absent, so no lease on it dies.
 event('Page.frameStartedLoading',{frameId:'frame-a'});
 assert.equal(browser.currentPageFrame('target-a'),cached);
 event('Page.navigatedWithinDocument',{frameId:'frame-a',url:'https://example.test/#x'});
 assert.equal(browser.currentPageFrame('target-a'),cached);
 // A history traversal navigates (the record retires) and then proves the document unchanged.
 event('Page.frameStartedNavigating',{frameId:'frame-a'});
 assert.equal(browser.currentPageFrame('target-a'),null);
 event('Page.navigatedWithinDocument',{frameId:'child',url:'https://example.test/#y'});
 assert.equal(browser.currentPageFrame('target-a'),null,'a subframe navigation proves nothing about the page');
 event('Page.navigatedWithinDocument',{frameId:'frame-a',url:'https://example.test/#y'});
 assert.equal(browser.currentPageFrame('target-a'),cached);
 event('Page.frameStartedNavigating',{frameId:'frame-a'});
 event('Page.frameNavigated',{frame:{id:'frame-a',loaderId:'doc-b'}});
 event('Page.navigatedWithinDocument',{frameId:'frame-a',url:'https://example.test/#z'});
 assert.equal(browser.currentPageFrame('target-a').frame.loaderId,'doc-b','a committed document is never replaced by the retired record');
 await browser.close();
});

test('MP-08/MP-10 the controller marks the commit and the load event of a top-level navigation for the kernel timing', async () => {
 const connection=new FakeConnection(),browser=new BrowserCdpClient({connectionFactory:async()=>connection}),marks=[];
 browser.pageTiming=(stage,started,ended)=>marks.push([stage,started,ended]);
 await browser.ensureConnection();const sessionId=await browser.ensureTargetSession(connection,'target-a');
 browser.recordConnectionEvent({method:'Page.frameNavigated',sessionId,params:{frame:{id:'child',parentId:'frame-a',loaderId:'c'}}});
 browser.recordConnectionEvent({method:'Page.loadEventFired',sessionId});
 assert.deepEqual(marks,[],'a subframe, or a load without a committed document, marks nothing');
 browser.recordConnectionEvent({method:'Page.frameNavigated',sessionId,params:{frame:{id:'frame-a',loaderId:'doc-a'}}});
 browser.recordConnectionEvent({method:'Page.loadEventFired',sessionId});
 assert.deepEqual(marks.map(([stage])=>stage),['page_commit','page_load']);
 const [[,committed,sameAt],[,from,loadedAt]]=marks;assert.equal(committed,sameAt);assert.equal(from,committed);assert(loadedAt>=from);
 await browser.close();
});

// r61 UX-09: a long poll in the old document learns of the commit when it happens, not when its own timer ends.
test('MP-08/MP-10 whenDocumentMoves resolves at the main-frame commit of another document and never for the same one', async () => {
 const connection=new FakeConnection(),browser=new BrowserCdpClient({connectionFactory:async()=>connection});
 await browser.ensureConnection();const sessionId=await browser.ensureTargetSession(connection,'target-a');
 browser.recordConnectionEvent({method:'Page.frameNavigated',sessionId,params:{frame:{id:'frame-a',loaderId:'doc-a'}}});
 let moved=false;const watch=browser.whenDocumentMoves('target-a','doc-a');watch.promise.then(()=>{moved=true});
 await new Promise(resolve=>setImmediate(resolve));assert.equal(moved,false);
 browser.recordConnectionEvent({method:'Page.frameNavigated',sessionId,params:{frame:{id:'frame-b',parentId:'frame-a',loaderId:'doc-child'}}});
 await new Promise(resolve=>setImmediate(resolve));assert.equal(moved,false,'a child frame is no document change');
 browser.recordConnectionEvent({method:'Page.frameNavigated',sessionId,params:{frame:{id:'frame-a',loaderId:'doc-b'}}});
 await new Promise(resolve=>setImmediate(resolve));assert.equal(moved,true);
 const stale=browser.whenDocumentMoves('target-a','doc-a');let at=false;stale.promise.then(()=>{at=true});await new Promise(resolve=>setImmediate(resolve));assert.equal(at,true,'an already moved document resolves at once');
 const cancelled=browser.whenDocumentMoves('target-a','doc-b');cancelled.cancel();assert.equal(browser.commitWaiters.size,0);
 watch.cancel();await browser.close();
});
