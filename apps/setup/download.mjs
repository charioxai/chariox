const fail = message => { throw new Error(message) }

export function publicUrl(value, allowPublicAssetQuery = false) {
  const url = new URL(value)
  if (url.username || url.password || (url.search && !allowPublicAssetQuery) || url.hash || !(url.protocol === "https:" || (url.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)))) fail("Setup requires HTTPS release/Cloud URLs (loopback allowed for tests)")
  return value.replace(/\/$/, "")
}
export async function download(url, path, limit) {
  const initial = new URL(publicUrl(url))
  let response
  for (let redirects = 0; ; redirects++) {
    response = await fetch(publicUrl(url, true), { redirect: "manual", signal: AbortSignal.timeout(300_000) })
    if (![301, 302, 303, 307, 308].includes(response.status)) break
    if (redirects >= 5 || !response.headers.get("location")) fail("release redirect refused")
    const next = new URL(response.headers.get("location"), url).href
    publicUrl(next, true)
    const target = new URL(next)
    if (initial.protocol === "https:" ? target.protocol !== "https:" : target.origin !== initial.origin) fail("release redirect transport refused")
    await response.body?.cancel()
    url = next
  }
  if (!response.ok || !response.body) fail("release download failed")
  if (Number(response.headers.get("content-length")) > limit) fail("release download exceeded limit")
  const { open } = await import("node:fs/promises")
  const file = await open(path, "wx", 0o600)
  let count = 0
  try {
    for await (const chunk of response.body) { count += chunk.length; if (count > limit) fail("release download exceeded limit"); await file.write(chunk) }
  } finally { await file.close() }
}
