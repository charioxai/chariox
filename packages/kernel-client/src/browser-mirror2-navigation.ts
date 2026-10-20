// MP-08/MP-10/MP-11: bounded mirror snapshots may contain only a document's head.
// Prepare the next script-free document without exposing that empty prefix. The
// prior, inert frame stays until the continuation has something visible to paint.
import { mirror2SandboxCsp } from './browser-mirror2-security.js'

export function hasVisibleMirrorContent(doc: Document): boolean {
  if (!doc.body || !doc.defaultView) return false
  const view = doc.defaultView
  const scrolling = doc.scrollingElement
  if (scrolling && (scrolling.scrollHeight > view.innerHeight || scrolling.scrollWidth > view.innerWidth)) return true
  const visible = (element: Element, rect: DOMRect): boolean => {
    if (rect.width < 2 || rect.height < 2 || rect.right <= 0 || rect.bottom <= 0 || rect.left >= view.innerWidth || rect.top >= view.innerHeight) return false
    const style = view.getComputedStyle(element)
    if (style.visibility !== 'visible' || style.display === 'none' || Number(style.opacity) === 0) return false
    const x = Math.max(0, Math.min(view.innerWidth - 1, (Math.max(0, rect.left) + Math.min(view.innerWidth, rect.right)) / 2))
    const y = Math.max(0, Math.min(view.innerHeight - 1, (Math.max(0, rect.top) + Math.min(view.innerHeight, rect.bottom)) / 2))
    for (let ancestor: Element | null = element.parentElement; ancestor; ancestor = ancestor.parentElement) if (Number(view.getComputedStyle(ancestor).opacity) === 0) return false
    const root = element.getRootNode() as Document | ShadowRoot
    const hit = 'elementFromPoint' in root ? root.elementFromPoint(x, y) : doc.elementFromPoint(x, y)
    return !!hit && (element === hit || element.contains(hit))
  }
  const content = (element: Element): boolean => {
    const style = view.getComputedStyle(element)
    const opaque = style.backgroundColor !== 'rgba(0, 0, 0, 0)' && style.backgroundColor !== 'rgb(255, 255, 255)'
    return (['input', 'textarea', 'select', 'button', 'img', 'svg', 'canvas', 'video', 'iframe'].includes(element.localName)
      || opaque || style.backgroundImage !== 'none' || element.scrollHeight > element.clientHeight || element.scrollWidth > element.clientWidth) && visible(element, element.getBoundingClientRect())
  }
  const text = (node: Node): boolean => {
    const parent = node.parentElement
    if (!parent || ['style', 'script'].includes(parent.localName) || !node.textContent?.trim()) return false
    const range = doc.createRange(); range.selectNodeContents(node)
    return visible(parent, range.getBoundingClientRect())
  }
  // Bounded work only while a document is being prepared, never on steady-state
  // deltas. A clipped skip link or a hidden loader cannot release the old page.
  const walkers = [doc.createTreeWalker(doc.body, 5 /* elements + text */)]
  for (let i = 0; i < 512 && walkers.length; i++) {
    const node = walkers.at(-1)!.nextNode()
    if (!node) { walkers.pop(); continue }
    if (node.nodeType === 3) { if (text(node)) return true }
    else {
      const element = node as Element
      if (content(element)) return true
      if (element.shadowRoot) walkers.push(doc.createTreeWalker(element.shadowRoot, 5))
    }
  }
  // Fragment/scroll restoration may put the visible content beyond that prefix.
  // Sample its viewport instead of walking an unbounded, already-continued tree.
  for (const y of [8, 24, 64, 160, 320, 600]) for (const x of [8, 64, 160, 320, 640, 960, 1200]) {
    const element = doc.elementFromPoint(Math.min(x, view.innerWidth - 1), Math.min(y, view.innerHeight - 1))
    if (!element || element === doc.body || element === doc.documentElement) continue
    if (content(element)) return true
    let count = 0
    for (const child of element.childNodes) { if (++count > 32) break; if (child.nodeType === 3 && text(child)) return true }
  }
  return false
}

// BROWSER-UX-09: the share of a 16x10 viewport grid that lands on content (text-bearing,
// replaced or painted boxes). A continuation's lone "Skip to content" link covers ~nothing.
export function mirrorContentCoverage(doc: Document): number {
  const view = doc.defaultView
  if (!doc.body || !view) return 0
  let hits = 0
  for (let row = 0; row < 10; row++) for (let column = 0; column < 16; column++) {
    const element = doc.elementFromPoint((column + 0.5) * view.innerWidth / 16, (row + 0.5) * view.innerHeight / 10)
    if (!element || element === doc.body || element === doc.documentElement) continue
    const style = view.getComputedStyle(element)
    if (['input', 'textarea', 'select', 'button', 'img', 'svg', 'canvas', 'video', 'iframe'].includes(element.localName) || style.backgroundImage !== 'none'
      || style.backgroundColor !== 'rgba(0, 0, 0, 0)' && style.backgroundColor !== 'rgb(255, 255, 255)'
      || [...element.childNodes].some(child => child.nodeType === 3 && child.textContent!.trim())) hits++
  }
  return hits / 160
}
const MEANINGFUL_COVERAGE = 0.05, SPARSE_HOLD_MS = 2000

export class Mirror2Navigation {
  frame: HTMLIFrameElement
  loaded: Promise<void>
  painted = false
  readyToPresent = false
  private filled = false
  private filledAt = 0
  private deferred = false
  private held: HTMLIFrameElement | undefined
  private cancelLoad: (() => void) | undefined
  private spare: [HTMLIFrameElement, Promise<void>] | undefined
  private cancelSpareLoad: (() => void) | undefined
  private emptyTimer: ReturnType<typeof setTimeout> | undefined
  private sparseTimer: ReturnType<typeof setTimeout> | undefined
  private cancelEmpty(): void { clearTimeout(this.emptyTimer); this.emptyTimer = undefined }
  private cancelSparse(): void { clearTimeout(this.sparseTimer); this.sparseTimer = undefined }
  constructor(private container: HTMLElement, private onEmptyPaint?: () => void) { [this.frame, this.loaded] = this.create() }
  private create(spare = false): [HTMLIFrameElement, Promise<void>] {
    const frame = this.container.ownerDocument.createElement('iframe')
    frame.setAttribute('sandbox', 'allow-same-origin')
    frame.setAttribute('referrerpolicy', 'no-referrer')
    frame.setAttribute('title', 'Mirrored kernel browser page')
    // The viewer's page area sets the layout size (protocol 560/168 layout_width/height); canonical when unset.
    frame.style.cssText = 'width:var(--mirror-frame-width,1280px);height:var(--mirror-frame-height,800px);border:0;display:block;visibility:hidden;background:white'
    if (spare) { frame.inert = true; frame.style.display = 'none' }
    frame.srcdoc = `<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="${mirror2SandboxCsp}"></head><body></body></html>`
    const loaded = new Promise<void>(resolve => {
      const finish = () => { frame.removeEventListener('load', finish); resolve() }
      if (spare) this.cancelSpareLoad = finish; else this.cancelLoad = finish
      frame.addEventListener('load', finish, { once: true })
    })
    this.container.append(frame)
    return [frame, loaded]
  }
  async prepare(): Promise<void> {
    this.cancelEmpty(); this.cancelSparse()
    this.cancelLoad?.()
    if (this.painted) { this.held?.remove(); this.held = this.frame; this.held.inert = true }
    else this.frame.remove() // a second navigation retires only its unpainted prefix
    this.painted = false; this.readyToPresent = false; this.filled = false; this.filledAt = 0
    if (this.spare) {
      ;[this.frame, this.loaded] = this.spare; this.spare = undefined
      this.cancelLoad = this.cancelSpareLoad; this.cancelSpareLoad = undefined
      this.frame.inert = false; this.frame.style.display = 'block'
    } else [this.frame, this.loaded] = this.create()
    await this.loaded
  }
  present(complete = false): void {
    if (!complete) this.cancelEmpty()
    // Native video owns paint while incomplete continuation records arrive.
    // Defer visibility/layout until completion can actually release that view.
    if (this.painted || !this.frame.contentDocument || this.deferred && !complete) return
    if (!hasVisibleMirrorContent(this.frame.contentDocument)) {
      // MP-08/MP-10/MP-11: a body can intentionally stay empty. Bound its
      // paint hold only after the kernel confirms the initial serialization
      // has drained and the native document has completed.
      if (complete && this.frame.contentDocument.body && this.emptyTimer === undefined) {
        const frame = this.frame
        this.emptyTimer = setTimeout(() => {
          this.emptyTimer = undefined
          if (this.frame === frame && frame.contentDocument?.body) { this.reveal(); this.onEmptyPaint?.() }
        }, 2000)
      }
      return
    }
    // An unfinished document holds the prior page until it covers some of the viewport;
    // a sparse page (or a slow continuation) still presents after a bounded wait.
    if (!complete && mirrorContentCoverage(this.frame.contentDocument) < MEANINGFUL_COVERAGE) {
      if (this.sparseTimer === undefined) {
        const frame = this.frame
        this.sparseTimer = setTimeout(() => {
          this.sparseTimer = undefined
          if (this.frame === frame && !this.painted && frame.contentDocument && hasVisibleMirrorContent(frame.contentDocument)) { this.reveal(); this.onEmptyPaint?.() }
        }, SPARSE_HOLD_MS)
      }
      return
    }
    this.reveal()
  }
  // A held, still-parsing document is ready once content continues below the fold (so the viewport is fully
  // serialized, in document order) and covers the viewport. Evaluated at most every 250 ms: it lays out a growing tree.
  viewportFilled(): boolean {
    if (this.filled) return true
    const doc = this.frame.contentDocument, view = doc?.defaultView, now = performance.now()
    if (!this.deferred || !doc || !view || now - this.filledAt < 250) return false
    this.filledAt = now
    if ((doc.scrollingElement?.scrollHeight ?? 0) <= view.innerHeight || !hasVisibleMirrorContent(doc) || mirrorContentCoverage(doc) < MEANINGFUL_COVERAGE) return false
    this.filled = true; this.reveal()
    return true
  }
  get holding(): boolean { return this.deferred }
  hold(): void { this.deferred = true }
  release(): void { this.deferred = false; if (this.readyToPresent) this.reveal() }
  private reveal(): void {
    this.readyToPresent = true
    if (this.deferred) return
    this.cancelEmpty(); this.cancelSparse()
    this.frame.style.visibility = 'visible'; this.painted = true
    this.held?.remove(); this.held = undefined
    this.spare ??= this.create(true)
  }
  private dropSpare(): void {
    this.cancelSpareLoad?.(); this.cancelSpareLoad = undefined
    this.spare?.[0].remove(); this.spare = undefined
  }
  preserve(): void {
    this.dropSpare()
    this.cancelEmpty(); this.cancelSparse()
    this.cancelLoad?.()
    if (this.held) { this.frame.remove(); this.frame = this.held; this.held = undefined; this.painted = true }
    this.frame.inert = true; this.frame.style.pointerEvents = 'none'
  }
  close(): void { this.dropSpare(); this.cancelEmpty(); this.cancelSparse(); this.cancelLoad?.(); this.frame.remove(); this.held?.remove(); this.held = undefined }
}
