import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import http from "node:http"
import { spawn } from "node:child_process"
import { setTimeout as sleep } from "node:timers/promises"
import { startActiveSoakFixture, waitForFixtureMarker } from "./browser-computer-soak-fixture.mjs"

// MP-08 / MP-10: opt-in, installed sandboxed Chromium only; no downloads.
assert.ok(process.env.CHARIOX_TEST_CHROMIUM, "set CHARIOX_TEST_CHROMIUM to an installed Chromium")
const controllerRoot = process.env.CHARIOX_TEST_CONTROLLER_ROOT
  ?? new URL("../../../kernel/slice-linux-docker/docker/", import.meta.url).href
const { BrowserCdpClient } = await import(`${controllerRoot}browser-controller-cdp.mjs`)
const { handleBrowserControllerRequest } = await import(`${controllerRoot}browser-controller.mjs`)

test("real Controller fill completes before async fixture acknowledgement; soak waits without replay", { timeout: 30_000 }, async () => {
  const fixture = await startActiveSoakFixture()
  const profile = await mkdtemp(path.join(os.tmpdir(), "chariox-soak-fixture-test-"))
  let chromium
  let browserExit
  let proxy
  let controller
  let releaseMark
  let releaseTimer
  let markRequests = 0
  let sequence = 0
  try {
    const gate = new Promise((resolve) => { releaseMark = resolve })
    proxy = http.createServer(async (request, response) => {
      if (request.url.startsWith("/mark?")) {
        markRequests += 1
        await gate
      }
      const upstream = await fetch(new URL(request.url, fixture.url))
      response.writeHead(upstream.status, { "content-type": upstream.headers.get("content-type") ?? "text/plain" })
      response.end(await upstream.text())
    })
    await new Promise((resolve) => proxy.listen(0, "127.0.0.1", resolve))
    const url = `http://127.0.0.1:${proxy.address().port}/`
    chromium = spawn(process.env.CHARIOX_TEST_CHROMIUM, [
      `--user-data-dir=${profile}`, "--remote-debugging-port=0", "--remote-debugging-address=127.0.0.1",
      "--password-store=basic", "--disable-dev-shm-usage", "--disable-gpu", "--no-first-run",
      "--no-default-browser-check", "--disable-sync", "--window-size=800,600",
      ...(!process.env.DISPLAY ? ["--headless=new"] : []), url,
    ], { stdio: ["ignore", "ignore", "inherit"] })
    browserExit = new Promise((resolve, reject) => { chromium.once("exit", resolve); chromium.once("error", reject) })
    let debugPort
    for (let attempt = 0; attempt < 200 && !debugPort; attempt += 1) {
      debugPort = await readFile(path.join(profile, "DevToolsActivePort"), "utf8")
        .then((text) => Number(text.split("\n")[0])).catch(() => null)
      if (!debugPort) await sleep(50)
    }
    assert.ok(debugPort, "sandboxed Chromium did not expose its debugging endpoint")
    controller = new BrowserCdpClient({ debuggerEndpoint: `http://127.0.0.1:${debugPort}` })
    const request = async (method, params) => {
      const response = await handleBrowserControllerRequest({ id: ++sequence, method, params }, { browser: controller })
      assert.equal(response.ok, true, JSON.stringify(response.error))
      return response.result
    }
    const viewport = {
      css_width: 800, css_height: 600, device_scale_factor: 1,
      desktop_pixel_width: 800, desktop_pixel_height: 600,
    }
    const { tabs } = await request("browser.reconcile", { viewport })
    const tab = tabs.find((entry) => entry.url === url)
    assert.ok(tab)
    const snapshot = await request("browser.snapshot", tab)
    const field = snapshot.accessibility_nodes.find((entry) => !entry.ignored && entry.role === "textbox" && entry.name.trim() === "Soak marker")
    assert.ok(field?.node_ref)
    const marker = "SOAK-00000536"
    await request("browser.action", { ...tab, node_ref: field.node_ref, action: { kind: "fill", text: marker } })
    const afterFill = await request("browser.reconcile", { viewport })
    assert.equal(afterFill.tabs.find((entry) => entry.target_id === tab.target_id)?.title, marker,
      "the exact input handler updated the DOM title")
    const immediate = await fetch(`${url}health`).then((response) => response.text())
    assert.equal(immediate, "SOAK-00000000", "old single-read assertion deterministically observes stale server state")
    console.log(JSON.stringify({ mp: ["MP-08", "MP-10"], fillReturned: true, domMarker: marker, immediateServerMarker: immediate }))
    releaseTimer = setTimeout(releaseMark, 250)
    const proof = await waitForFixtureMarker(url, marker)
    assert.ok(proof.attempts > 1)
    assert.equal(markRequests, 1, "read polling must not repeat browser actions")
    console.log(JSON.stringify({ mp: ["MP-08", "MP-10"], acknowledgement: proof, markRequests }))
  } finally {
    clearTimeout(releaseTimer)
    releaseMark?.()
    await controller?.close()
    if (chromium && chromium.exitCode === null) chromium.kill("SIGTERM")
    await browserExit
    if (proxy) await new Promise((resolve) => proxy.close(resolve))
    await fixture.close()
    await rm(profile, { recursive: true, force: true })
  }
})
