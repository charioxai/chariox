// MP-08 / MP-10 / MP-11 (PANE-09, browser observation protection): images in
// rendered Markdown. A pane never fetches a third-party URL on its own:
// - `data:image/png|jpeg|gif|webp` renders inline;
// - workspace paths load through the kernel (GetWorkspaceFileContent over the
//   relay), so bytes come from the user's own kernel, never the network;
// - remote http(s) images show a card and load only after a click; the
//   approval holds for that exact URL for the rest of the page session.

export type MarkdownWorkspaceImageLoader = (element: HTMLElement, reference: string) => Promise<MarkdownWorkspaceImage>
export type MarkdownWorkspaceImage =
  | { readonly kind: "loaded"; readonly src: string; readonly sizeBytes: number }
  | { readonly kind: "unavailable"; readonly message: string }

type ImageSource =
  | { readonly kind: "inline"; readonly src: string }
  | { readonly kind: "remote"; readonly src: string; readonly host: string; readonly file: string }
  | { readonly kind: "workspace"; readonly path: string; readonly file: string }
  | { readonly kind: "unsupported"; readonly src: string }

const inlineImagePattern = /^data:image\/(?:png|jpe?g|gif|webp);base64,[a-z0-9+/=\s]+$/i
const imageFilePattern = /\.(?:png|jpe?g|gif|webp)$/i
type ImageState = {
  readonly approvedRemoteSources: Set<string>
  workspaceImageLoader: MarkdownWorkspaceImageLoader | null
  scanScheduled: boolean
}

// The React shell and terminal runtime bundle this module separately. Keep
// loader ownership and click consent on their shared document, just like the
// delegated interaction listener; a second bundle must not lose either.
const imageStateKey = Symbol.for("chariox.markdown.image-state.v1")
function imageState(target: Document | undefined = globalThis.document): ImageState {
  const owner = (typeof document === "undefined" ? globalThis : target ?? document) as unknown as Record<symbol, ImageState | undefined>
  return owner[imageStateKey] ??= { approvedRemoteSources: new Set(), workspaceImageLoader: null, scanScheduled: false }
}

export function setMarkdownWorkspaceImageLoader(loader: MarkdownWorkspaceImageLoader | null): void {
  imageState().workspaceImageLoader = loader
  if (loader) scheduleMarkdownImageScan()
}

export function renderMarkdownImage(rawSrc: string, alt: string, escapeHtml: (value: string) => string): string {
  const source = classifyImageSource(rawSrc)
  const label = alt.trim() || "image"
  if (source.kind === "inline") {
    return `<span class="freeform-md-image" data-md-image="inline" data-md-image-state="loaded"><img class="freeform-md-img" src="${escapeHtml(source.src)}" alt="${escapeHtml(label)}" title="${escapeHtml(`${label} · ${formatBytes(inlineImageBytes(source.src))}`)}"></span>`
  }
  if (source.kind === "remote") {
    scheduleMarkdownImageScan()
    return `<span class="freeform-md-image" data-md-image="remote" data-md-image-state="blocked" data-md-image-src="${escapeHtml(source.src)}"><span class="freeform-md-image-card"><span class="freeform-md-image-alt">${escapeHtml(label)}</span><span class="freeform-md-image-meta">${escapeHtml([source.host, source.file].filter(Boolean).join(" · "))}</span><span class="freeform-md-image-actions"><button type="button" class="freeform-md-image-load" data-markdown-image-load aria-label="Load image from ${escapeHtml(source.host)}">load image</button><a class="freeform-md-link" href="${escapeHtml(source.src)}" target="_blank" rel="noopener noreferrer nofollow">open</a></span></span></span>`
  }
  if (source.kind === "workspace") {
    scheduleMarkdownImageScan()
    return `<span class="freeform-md-image" data-md-image="workspace" data-md-image-state="pending" data-md-image-path="${escapeHtml(source.path)}"><span class="freeform-md-image-card"><span class="freeform-md-image-alt">${escapeHtml(label)}</span><span class="freeform-md-image-meta">workspace · ${escapeHtml(source.file)}</span><span class="freeform-md-image-status">loading…</span></span></span>`
  }
  return `<span class="freeform-md-image" data-md-image="unsupported" data-md-image-state="unsupported"><span class="freeform-md-image-card"><span class="freeform-md-image-alt">${escapeHtml(label)}</span><span class="freeform-md-image-meta">${escapeHtml(source.src || "no source")}</span></span></span>`
}

export function classifyImageSource(rawSrc: string): ImageSource {
  const src = rawSrc.trim()
  if (inlineImagePattern.test(src)) return { kind: "inline", src: src.replace(/\s+/g, "") }
  if (/^https?:\/\//i.test(src)) {
    try {
      const url = new URL(src)
      return { kind: "remote", src: url.href, host: url.host, file: decodeURIComponent(url.pathname.split("/").pop() ?? "") }
    } catch {
      return { kind: "unsupported", src }
    }
  }
  let path = src.replace(/^file:\/\//i, "").replace(/[?#].*$/, "")
  try { path = decodeURIComponent(path) } catch { return { kind: "unsupported", src } }
  if (!/^[a-z][a-z0-9+.-]*:/i.test(path) && !path.startsWith("//") && imageFilePattern.test(path)) {
    return { kind: "workspace", path, file: path.split("/").pop() ?? path }
  }
  return { kind: "unsupported", src }
}

// One click on "load image": approve this exact URL and load every card for it.
export function handleMarkdownImageLoadClick(button: HTMLElement): boolean {
  const container = button.closest<HTMLElement>("[data-md-image='remote']")
  const src = container?.dataset.mdImageSrc
  if (!container || !src) return false
  imageState(container.ownerDocument).approvedRemoteSources.add(src)
  const owner = container.ownerDocument
  for (const element of owner.querySelectorAll<HTMLElement>("[data-md-image='remote'][data-md-image-state='blocked']")) {
    if (element.dataset.mdImageSrc === src) loadRemoteImage(element, src)
  }
  return true
}

export function handleMarkdownImageClick(image: HTMLImageElement): boolean {
  const container = image.closest<HTMLElement>(".freeform-md-image")
  if (!container) return false
  container.dataset.mdImageExpanded = container.dataset.mdImageExpanded === "true" ? "false" : "true"
  return true
}

export function scheduleMarkdownImageScan(): void {
  const target = globalThis.document
  if (!target) return
  const state = imageState(target)
  if (state.scanScheduled) return
  state.scanScheduled = true
  setTimeout(() => {
    state.scanScheduled = false
    scanMarkdownImages(target)
  }, 0)
}

export function scanMarkdownImages(target: Document): void {
  const { approvedRemoteSources, workspaceImageLoader } = imageState(target)
  for (const element of target.querySelectorAll<HTMLElement>("[data-md-image='remote'][data-md-image-state='blocked']")) {
    const src = element.dataset.mdImageSrc
    if (src && approvedRemoteSources.has(src)) loadRemoteImage(element, src)
  }
  if (!workspaceImageLoader) return
  for (const element of target.querySelectorAll<HTMLElement>("[data-md-image='workspace'][data-md-image-state='pending']")) {
    void loadWorkspaceImage(element, workspaceImageLoader)
  }
}

export function sniffImageMime(base64: string): string | null {
  const head = base64.slice(0, 24)
  if (head.startsWith("iVBORw0KGgo")) return "image/png"
  if (head.startsWith("/9j/")) return "image/jpeg"
  if (head.startsWith("R0lGOD")) return "image/gif"
  if (head.startsWith("UklGR") && atobSafe(head).slice(8, 12) === "WEBP") return "image/webp"
  return null
}

export function resetMarkdownImagesForTests(): void {
  const state = imageState()
  state.approvedRemoteSources.clear()
  state.workspaceImageLoader = null
  state.scanScheduled = false
}

function loadRemoteImage(container: HTMLElement, src: string): void {
  const owner = container.ownerDocument
  container.dataset.mdImageState = "loading"
  const image = owner.createElement("img")
  image.className = "freeform-md-img"
  image.alt = container.querySelector(".freeform-md-image-alt")?.textContent ?? "image"
  image.referrerPolicy = "no-referrer"
  image.decoding = "async"
  image.addEventListener("load", () => {
    container.dataset.mdImageState = "loaded"
  }, { once: true })
  image.addEventListener("error", () => {
    container.dataset.mdImageState = "failed"
    image.remove()
    setStatus(container, "image failed to load")
  }, { once: true })
  image.src = src
  container.prepend(image)
  container.querySelector(".freeform-md-image-load")?.remove()
}

async function loadWorkspaceImage(container: HTMLElement, loader: MarkdownWorkspaceImageLoader): Promise<void> {
  const path = container.dataset.mdImagePath ?? ""
  container.dataset.mdImageState = "loading"
  const result = await loader(container, path).catch(() => ({ kind: "unavailable", message: "image unavailable" } as const))
  if (result.kind !== "loaded") {
    container.dataset.mdImageState = "unavailable"
    setStatus(container, result.message)
    return
  }
  const image = container.ownerDocument.createElement("img")
  image.className = "freeform-md-img"
  image.alt = container.querySelector(".freeform-md-image-alt")?.textContent ?? "image"
  image.title = `${image.alt} · ${formatBytes(result.sizeBytes)}`
  image.src = result.src
  container.replaceChildren(image)
  container.dataset.mdImageState = "loaded"
}

function setStatus(container: HTMLElement, message: string): void {
  const card = container.querySelector(".freeform-md-image-card")
  if (!card) return
  let status = card.querySelector(".freeform-md-image-status")
  if (!status) {
    status = container.ownerDocument.createElement("span")
    status.className = "freeform-md-image-status"
    card.append(status)
  }
  status.textContent = message
}

function inlineImageBytes(src: string): number {
  const data = src.slice(src.indexOf(",") + 1)
  return Math.floor(data.length * 3 / 4) - (data.endsWith("==") ? 2 : data.endsWith("=") ? 1 : 0)
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

function atobSafe(value: string): string {
  try {
    return atob(value.slice(0, 16))
  } catch {
    return ""
  }
}
