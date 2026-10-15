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

export class Mirror2Navigation {
  frame: HTMLIFrameElement
  loaded: Promise<void>
  painted = false
  private held: HTMLIFrameElement | undefined
  private cancelLoad: (() => void) | undefined
  constructor(private container: HTMLElement) { [this.frame, this.loaded] = this.create() }
  private create(): [HTMLIFrameElement, Promise<void>] {
    const frame = this.container.ownerDocument.createElement('iframe')
    frame.setAttribute('sandbox', 'allow-same-origin')
    frame.setAttribute('referrerpolicy', 'no-referrer')
    frame.setAttribute('title', 'Mirrored kernel browser page')
    frame.style.cssText = 'width:1280px;height:800px;border:0;display:block;visibility:hidden'
    frame.srcdoc = `<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="${mirror2SandboxCsp}"></head><body></body></html>`
    const loaded = new Promise<void>(resolve => {
      const finish = () => { frame.removeEventListener('load', finish); resolve() }
      this.cancelLoad = finish
      frame.addEventListener('load', finish, { once: true })
    })
    this.container.append(frame)
    return [frame, loaded]
  }
  async prepare(): Promise<void> {
    this.cancelLoad?.()
    if (this.painted) { this.held?.remove(); this.held = this.frame; this.held.inert = true }
    else this.frame.remove() // a second navigation retires only its unpainted prefix
    this.painted = false
    ;[this.frame, this.loaded] = this.create()
    await this.loaded
  }
  present(): void {
    if (this.painted || !this.frame.contentDocument || !hasVisibleMirrorContent(this.frame.contentDocument)) return
    this.frame.style.visibility = 'visible'; this.painted = true
    this.held?.remove(); this.held = undefined
  }
  preserve(): void {
    this.cancelLoad?.()
    if (this.held) { this.frame.remove(); this.frame = this.held; this.held = undefined; this.painted = true }
    this.frame.inert = true; this.frame.style.pointerEvents = 'none'
  }
  close(): void { this.cancelLoad?.(); this.frame.remove(); this.held?.remove(); this.held = undefined }
}
