// The Room browser bar: ordinary Tabs' windows are fullscreen, like App views,
// so the page covers the desktop and viewers see what agents see. Showing the
// bar maximizes them instead, with Chromium's tab strip and address bar.
// Windows holding an App view stay as the App view keeps them (fullscreen).
//
// A window is set only when it is new or the bar changed since it was last set
// (`applied`, window id -> the bar state last applied): a page that enters its
// own HTML fullscreen (a video) is not forced back on the next reconcile.
export async function applyBrowserBar(connection, pages, appTargetIds, visible, applied = new Map()) {
  const wanted = visible ? "maximized" : "fullscreen";
  // Tabs and windows can close at any point during a reconcile: skip them.
  const quietly = async (fn) => {
    try {
      return await fn();
    } catch {
      return null;
    }
  };
  const windowOf = async (targetId) =>
    (await quietly(() => connection.send("Browser.getWindowForTarget", { targetId })))?.windowId ?? null;
  const appWindows = new Set();
  for (const targetId of appTargetIds) {
    const windowId = await windowOf(targetId);
    if (windowId !== null) appWindows.add(windowId);
  }
  const windows = new Set();
  for (const page of pages) {
    if (appTargetIds.has(page.targetId)) continue;
    const windowId = await windowOf(page.targetId);
    if (windowId === null || appWindows.has(windowId) || windows.has(windowId)) continue;
    windows.add(windowId);
    if (applied.get(windowId) === visible) continue;
    const bounds = (await quietly(() => connection.send("Browser.getWindowBounds", { windowId })))?.bounds;
    if (!bounds) continue;
    const state = bounds.windowState;
    // A minimized window is set once it is restored: not recorded until then.
    if (state === "minimized") continue;
    if (state !== wanted) {
      // Chromium only leaves fullscreen or maximized through the normal state.
      if (state !== "normal") {
        await quietly(() => connection.send("Browser.setWindowBounds", { windowId, bounds: { windowState: "normal" } }));
      }
      const set = await quietly(() => connection.send("Browser.setWindowBounds", { windowId, bounds: { windowState: wanted } }));
      if (set === null) continue;
    }
    applied.set(windowId, visible);
  }
  // Forget windows that closed.
  for (const windowId of applied.keys()) {
    if (!windows.has(windowId)) applied.delete(windowId);
  }
  return windows.size;
}
