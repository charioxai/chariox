import assert from "node:assert/strict"

// Observe only this synthetic fixture through the existing slice CDP client.
// Native mousedown supplies the measured CSS-content to desktop offset.
export async function observeRoomGestureFixture(connection, fixtureOrigin) {
  const { targetInfos } = await connection.send("Target.getTargets")
  const targets = targetInfos.filter(target => {
    if (target.type !== "page") return false
    try {
      const url = new URL(target.url)
      return url.origin === fixtureOrigin && url.pathname === "/click"
    } catch { return false }
  })
  if (targets.length !== 1) throw new Error("gesture fixture must have exactly one browser target")
  const { sessionId } = await connection.send("Target.attachToTarget", {
    targetId: targets[0].targetId, flatten: true,
  })
  try {
    const response = await connection.send("Runtime.evaluate", {
      expression: `(() => {
        const rect = selector => {
          const element = document.querySelector(selector);
          if (!element) return null;
          const { x, y, width, height } = element.getBoundingClientRect();
          return { x, y, width, height };
        };
        return {
          origin: window.charioxGestureOrigin ?? null,
          windowGeometry: [window.screenX, window.screenY, window.outerWidth, window.outerHeight],
          selection: rect("#web-selection"),
          scroller: rect("#web-scroller"),
          fixtureOrigin: location.origin,
          fixturePath: location.pathname
        };
      })()`,
      returnByValue: true,
    }, sessionId)
    if (response.exceptionDetails || !response.result?.value) throw new Error("gesture fixture observation failed")
    const observation = response.result.value
    if (observation.fixtureOrigin !== fixtureOrigin || observation.fixturePath !== "/click") {
      throw new Error("gesture fixture navigated during observation")
    }
    return observation
  } finally {
    await connection.send("Target.detachFromTarget", { sessionId })
  }
}

export function roomGestureObservationScript(fixtureOrigin) {
  const url = new URL(fixtureOrigin)
  assert.equal(url.protocol, "http:")
  assert.equal(url.origin, fixtureOrigin)
  return [
    'import { BrowserCdpClient } from "/opt/chariox-slice/browser-controller-cdp.mjs";',
    `const observe = ${observeRoomGestureFixture.toString()};`,
    "const client = new BrowserCdpClient({requestTimeoutMs:5000});",
    "try {",
    " const connection = await client.openConnection();",
    ` console.log(JSON.stringify(await observe(connection, ${JSON.stringify(fixtureOrigin)})));`,
    "} finally { client.close(); }",
  ].join("\n")
}

export function roomCompanionGestureTargets(initial, current, viewport) {
  assert.ok(initial?.origin, "gesture fixture has no trusted pointer origin")
  for (const value of [initial.origin.x, initial.origin.y, ...initial.windowGeometry ?? [], ...current.windowGeometry ?? []]) {
    assert.ok(Number.isFinite(value), "gesture geometry must be finite")
  }
  assert.equal(initial.windowGeometry?.length, 4, "initial window geometry is missing")
  assert.equal(current.windowGeometry?.length, 4, "current window geometry is missing")
  assert.deepEqual(current.windowGeometry, initial.windowGeometry, "browser window moved before Web gestures")
  const width = viewport?.desktop_pixel_width
  const height = viewport?.desktop_pixel_height
  assert.ok(Number.isFinite(width) && width > 0 && Number.isFinite(height) && height > 0,
    "canonical desktop viewport must have positive dimensions")
  const point = (rect, fractionX, fractionY) => {
    assert.ok(rect && [rect.x, rect.y, rect.width, rect.height].every(Number.isFinite), "gesture target rectangle is missing")
    assert.ok(rect.width > 0 && rect.height > 0, "gesture target rectangle is empty")
    const result = {
      x: Math.round(initial.origin.x + rect.x + rect.width * fractionX),
      y: Math.round(initial.origin.y + rect.y + rect.height * fractionY),
    }
    assert.ok(result.x >= 0 && result.x < width && result.y >= 0 && result.y < height,
      "gesture target is outside the desktop viewport")
    return result
  }
  return {
    drag: { start: point(current.selection, 0.1, 0.5), end: point(current.selection, 0.9, 0.5) },
    scroll: point(current.scroller, 0.5, 0.5),
  }
}
