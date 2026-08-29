export function browserPageTargets(targets) {
  return targets.filter((target) => target.type === "page" && target.webSocketDebuggerUrl)
}

export function fallbackBrowserPageTarget(pages) {
  return pages.find((target) => target.url?.includes("chariox-slice-screen-test"))
    ?? pages.find((target) => target.url?.startsWith("file:///workspace/"))
    ?? pages.find((target) => target.url && target.url !== "about:blank")
    ?? pages.at(-1)
    ?? null
}

export async function selectBrowserPageTarget(targets, isVisible) {
  const pages = browserPageTargets(targets)
  if (pages.length <= 1) {
    return pages[0] ?? null
  }
  const visibility = await Promise.all(pages.map(async (page) => {
    try {
      return await isVisible(page)
    } catch {
      return false
    }
  }))
  return pages.find((_, index) => visibility[index]) ?? fallbackBrowserPageTarget(pages)
}
