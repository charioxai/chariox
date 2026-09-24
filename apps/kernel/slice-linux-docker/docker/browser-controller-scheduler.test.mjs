import assert from "node:assert/strict";
import { PassThrough } from "node:stream";
import readline from "node:readline";
import test from "node:test";
import { readFileSync } from "node:fs";

import { BrowserControllerStdioServer } from "./browser-controller.mjs";
import { BrowserControllerRequestScheduler } from "./browser-controller-scheduler.mjs";

function deferred() {
  return Promise.withResolvers();
}

test("slice image and refresh overlay ship the scheduler dependency", () => {
  const dockerfile = readFileSync(new URL("./Dockerfile", import.meta.url), "utf8");
  const provisioner = readFileSync(new URL("../provision-linux-docker-slice.sh", import.meta.url), "utf8");
  assert.match(dockerfile, /COPY --chown=slice:slice .*browser-controller-scheduler\.mjs \/opt\/chariox-slice\/browser-controller-scheduler\.mjs/);
  assert.match(provisioner, /copy_required_slice_overlay .*browser-controller-scheduler\.mjs/);
});

function startController(browser) {
  const input = new PassThrough();
  const output = new PassThrough();
  const lines = readline.createInterface({ input: output, crlfDelay: Infinity });
  const responses = new Map();
  lines.on("line", (line) => {
    const response = JSON.parse(line);
    responses.get(response.id)?.resolve(response);
  });
  const running = new BrowserControllerStdioServer({ input, output, browser }).run();
  return {
    request(id, method, params = {}) {
      const pending = deferred();
      responses.set(id, pending);
      input.write(`${JSON.stringify({ id, method, params })}\n`);
      return pending.promise.finally(() => responses.delete(id));
    },
    async close() {
      input.end();
      await running;
      lines.close();
      output.end();
    },
  };
}

function actionRequest(targetId) {
  return { method: "browser.action", params: { target_id: targetId } };
}

test("stdio controller overlaps different tabs and observations while preserving same-tab order", { timeout: 5_000 }, async () => {
  const firstStarted = deferred();
  const releaseFirst = deferred();
  const sameTabSecondStarted = deferred();
  const otherTabStarted = deferred();
  const snapshotStarted = deferred();
  const starts = [];
  const browser = {
    async performAction({ target_id: targetId, action }) {
      starts.push(action.name);
      if (action.name === "first") {
        firstStarted.resolve();
        await releaseFirst.promise;
      } else if (action.name === "same-tab-second") {
        sameTabSecondStarted.resolve();
      } else if (action.name === "other-tab") {
        otherTabStarted.resolve();
      }
      return { target_id: targetId, action: action.name };
    },
    async snapshot({ target_id: targetId }) {
      snapshotStarted.resolve();
      return { target_id: targetId };
    },
  };
  const server = startController(browser);

  try {
    const first = server.request(1, "browser.action", {
      target_id: "tab-a", document_id: "doc-a", action: { name: "first" },
    });
    await firstStarted.promise;

    const sameTabSecond = server.request(2, "browser.action", {
      target_id: "tab-a", document_id: "doc-a", action: { name: "same-tab-second" },
    });
    const otherTab = server.request(3, "browser.action", {
      target_id: "tab-b", document_id: "doc-b", action: { name: "other-tab" },
    });
    const snapshot = server.request(4, "browser.snapshot", {
      target_id: "tab-a", document_id: "doc-a",
    });

    await Promise.all([otherTabStarted.promise, snapshotStarted.promise]);
    assert.equal(starts.includes("same-tab-second"), false, "the second tab-a mutation must wait");
    assert.equal((await otherTab).ok, true, "tab-b must finish while tab-a is held");
    assert.equal((await snapshot).ok, true, "observations may overlap a held mutation");
    assert.equal(starts.includes("same-tab-second"), false);

    releaseFirst.resolve();
    assert.equal((await first).ok, true);
    await sameTabSecondStarted.promise;
    assert.equal((await sameTabSecond).ok, true);
    assert.deepEqual(starts, ["first", "other-tab", "same-tab-second"]);
  } finally {
    releaseFirst.resolve();
    await server.close();
  }
});

test("a rejected operation releases its tab lane for the next request", { timeout: 5_000 }, async () => {
  const scheduler = new BrowserControllerRequestScheduler();
  const firstStarted = deferred();
  const failFirst = deferred();
  const secondStarted = deferred();
  const first = scheduler.schedule(actionRequest("tab-a"), async () => {
    firstStarted.resolve();
    await failFirst.promise;
  });
  const observedFailure = assert.rejects(first, /controlled failure/);
  await firstStarted.promise;

  const second = scheduler.schedule(actionRequest("tab-a"), () => {
    secondStarted.resolve();
    return "recovered";
  });
  failFirst.reject(new Error("controlled failure"));

  await observedFailure;
  await secondStarted.promise;
  assert.equal(await second, "recovered");
  await scheduler.drain();
});

test("browser-wide requests wait for prior tabs and fence later requests", { timeout: 5_000 }, async () => {
  const globalMethods = [
    "browser.reconcile",
    "browser.tab",
    "browser.downloads.configure",
    "browser.downloads.cancel",
    "browser.permission",
    "browser.cookies.import",
    "browser.cookies.recover",
    "shutdown",
  ];

  for (const method of globalMethods) {
    const scheduler = new BrowserControllerRequestScheduler();
    const beforeStarted = deferred();
    const releaseBefore = deferred();
    const barrierStarted = deferred();
    const releaseBarrier = deferred();
    const afterStarted = deferred();
    let barrierHasStarted = false;
    let afterHasStarted = false;

    const before = scheduler.schedule(actionRequest("tab-before"), async () => {
      beforeStarted.resolve();
      await releaseBefore.promise;
    });
    try {
      await beforeStarted.promise;
      const params = ["browser.tab", "browser.downloads.configure", "browser.permission"].includes(method)
        ? { target_id: "tab-global" }
        : {};
      const barrier = scheduler.schedule({ method, params }, async () => {
        barrierHasStarted = true;
        barrierStarted.resolve();
        await releaseBarrier.promise;
      });
      const after = scheduler.schedule(actionRequest("tab-after"), () => {
        afterHasStarted = true;
        afterStarted.resolve();
      });

      assert.equal(barrierHasStarted, false, `${method} must wait for earlier tab work`);
      assert.equal(afterHasStarted, false, `${method} must fence later tab work`);
      releaseBefore.resolve();
      await barrierStarted.promise;
      assert.equal(afterHasStarted, false, `${method} must remain exclusive while active`);
      releaseBarrier.resolve();
      await afterStarted.promise;
      await Promise.all([before, barrier, after]);
      await scheduler.drain();
    } finally {
      releaseBefore.resolve();
      releaseBarrier.resolve();
      await scheduler.drain();
    }
  }
});
