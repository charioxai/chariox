// The Room browser bar: ordinary Tabs' windows are fullscreen, like App views,
// so the page covers the desktop and viewers see what agents see. Showing the
// bar maximizes them instead, with Chromium's tab strip and address bar.
// Windows holding an App view stay as the App view keeps them (fullscreen).
export async function applyBrowserBar(connection, pages, appTargetIds, visible) {
  const wanted = visible ? "maximized" : "fullscreen";
  const windowOf = async (targetId) => {
    try {
      return (await connection.send("Browser.getWindowForTarget", { targetId })).windowId;
    } catch {
      return null; // The Tab closed meanwhile.
    }
  };
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
    const { bounds } = await connection.send("Browser.getWindowBounds", { windowId });
    const state = bounds?.windowState;
    if (state === wanted || state === "minimized") continue;
    // Chromium only leaves fullscreen or maximized through the normal state.
    if (state !== "normal") {
      await connection.send("Browser.setWindowBounds", { windowId, bounds: { windowState: "normal" } });
    }
    await connection.send("Browser.setWindowBounds", { windowId, bounds: { windowState: wanted } });
  }
  return windows.size;
}
