// MP-10: viewer wheel scrolling presents every frame. A precise wheel step (a
// trackpad or CDP pixel delta) scrolls natively in one jump; spaced steps (one
// per 50 ms) would then present one change per step. Here a step eases over
// the frames until the next is due, the way a browser animates a wheel tick;
// steps at frame rate or faster stay immediate. A mouse wheel's notches ease
// over about the browser's own tick animation (95% in ~180 ms) but continuously:
// a reversal or a sub-pixel tail does not stall the content for frames (the
// browser's per-tick curve presented ~55 of 60 frames at DPR1). The scroller is
// the one native chaining picks; zoom, page/line steps and snap or contained
// scrollers keep the native wheel. Positions end exactly where the native steps would.
const MAX_EASE_MS = 40
const NOTCH_EASE_MS = 60
// A notch: 120 per tick (X11, Chrome), 100 (Windows) or another line-sized step; the kernel's input classifies alike.
const notch = (delta: number): boolean => delta !== 0 && (delta % 120 === 0 || Math.abs(delta) >= 50)

type Axis = 'x' | 'y'
type Glide = { el: Element; x: number; y: number; rx: number; ry: number; tau: number; last: number }

const root = (el: Element): boolean => el === el.ownerDocument.scrollingElement || el === el.ownerDocument.documentElement || el === el.ownerDocument.body
const view = (el: Element): Window => el.ownerDocument.defaultView!
const position = (el: Element): [number, number] => root(el) ? [view(el).scrollX, view(el).scrollY] : [el.scrollLeft, el.scrollTop]
const limit = (el: Element): [number, number] => { const s = root(el) ? el.ownerDocument.scrollingElement ?? el : el; return [s.scrollWidth - s.clientWidth, s.scrollHeight - s.clientHeight] }
const scrollable = (overflow: string): boolean => overflow === 'auto' || overflow === 'scroll' || overflow === 'overlay'

export class Mirror2Wheel {
  private glides = new Map<Element, Glide>()
  private steps = new WeakMap<Element, number>() // the previous step's time per scroller
  private frame = 0
  constructor(private request: (fn: FrameRequestCallback) => number, private cancelFrame: (handle: number) => void) {}
  // True when this wheel event is taken (the caller prevents its default).
  take(event: WheelEvent, target: Element | null, now = performance.now()): boolean {
    if (!target || event.ctrlKey || event.deltaMode !== 0 || event.shiftKey && !event.deltaX) return false
    const dx = event.deltaX, dy = event.deltaY
    if (!dx && !dy) return false
    const el = this.scroller(target, dx, dy)
    if (!el) return false
    let glide = this.glides.get(el)
    if (glide) { const [x, y] = position(el); if (Math.abs(x - glide.x) >= 1 || Math.abs(y - glide.y) >= 1) { this.glides.delete(el); glide = undefined } }
    if (!glide) { const [x, y] = position(el); glide = { el, x, y, rx: 0, ry: 0, tau: 0, last: now - 16 }; this.glides.set(el, glide) }
    // Spacing distinguishes a notch from a large frame-rate trackpad delta.
    const interval = now - (this.steps.get(el) ?? -Infinity)
    const tau = interval <= 32 ? 0 : notch(dx) || notch(dy) ? NOTCH_EASE_MS : Math.min(MAX_EASE_MS, Math.max(0, interval - 16))
    this.steps.set(el, now)
    const [mx, my] = limit(el)
    if (!tau && (glide.rx || glide.ry)) {
      // Follow new precise input immediately; let the old tail finish on its existing curve.
      glide.x = Math.min(mx, Math.max(0, glide.x + dx)); glide.y = Math.min(my, Math.max(0, glide.y + dy))
      const options = { left: glide.x, top: glide.y, behavior: 'instant' as ScrollBehavior }
      if (root(el)) view(el).scrollTo(options); else el.scrollTo(options)
      const [x, y] = position(el); glide.x = x; glide.y = y
      glide.rx = Math.min(mx, Math.max(0, x + glide.rx)) - x; glide.ry = Math.min(my, Math.max(0, y + glide.ry)) - y
      this.frame ||= this.request(() => this.tick())
    } else {
      glide.tau = tau
      glide.rx = Math.min(mx, Math.max(0, glide.x + glide.rx + dx)) - glide.x
      glide.ry = Math.min(my, Math.max(0, glide.y + glide.ry + dy)) - glide.y
      if (!glide.tau) this.step(glide, now)
      else this.frame ||= this.request(() => this.tick())
    }
    return true
  }
  // Preserve only the top document's remaining wheel intent across its reset.
  rootTarget(doc: Document): [number, number] | null {
    for (const glide of this.glides.values()) if (glide.el.ownerDocument === doc && root(glide.el)) return [glide.x + glide.rx, glide.y + glide.ry]
    return null
  }
  // A position the kernel set (or a reset) ends that scroller's glide.
  cancel(el?: Element): void { if (el) this.glides.delete(el); else this.glides.clear() }
  close(): void { this.glides.clear(); if (this.frame) this.cancelFrame(this.frame); this.frame = 0 }
  private tick(now = performance.now()): void {
    this.frame = 0
    for (const glide of this.glides.values()) this.step(glide, now)
    if (this.glides.size) this.frame = this.request(() => this.tick())
  }
  private step(glide: Glide, now: number): void {
    const { el } = glide
    if (!el.isConnected) { this.glides.delete(el); return }
    // A native keyboard/scrollbar/layout move owns the position before our write.
    const [liveX, liveY] = position(el)
    if (Math.abs(liveX - glide.x) >= 1 || Math.abs(liveY - glide.y) >= 1) { this.glides.delete(el); return }
    const k = glide.tau ? 1 - Math.exp(-Math.max(0, now - glide.last) / glide.tau) : 1
    glide.last = now
    const mx = Math.abs(glide.rx * (1 - k)) < 0.5 ? glide.rx : glide.rx * k, my = Math.abs(glide.ry * (1 - k)) < 0.5 ? glide.ry : glide.ry * k
    glide.x += mx; glide.y += my; glide.rx -= mx; glide.ry -= my
    const options = { left: glide.x, top: glide.y, behavior: 'instant' as ScrollBehavior }
    if (root(el)) view(el).scrollTo(options); else el.scrollTo(options)
    // Something else moved it (a boundary, layout, the page): continue from where it is.
    const [x, y] = position(el)
    if (Math.abs(x - glide.x) >= 1) { glide.x = x; glide.rx = 0 }
    if (Math.abs(y - glide.y) >= 1) { glide.y = y; glide.ry = 0 }
    if (!glide.rx && !glide.ry) this.glides.delete(el)
  }
  // Native chaining: the innermost scroll container that can still move this way; the document's viewport last.
  private scroller(target: Element, dx: number, dy: number): Element | null {
    const can = (el: Element, axis: Axis, d: number): boolean => {
      const glide = this.glides.get(el), [px, py] = position(el), [mx, my] = limit(el)
      const at = axis === 'x' ? (glide ? glide.x + glide.rx : px) : (glide ? glide.y + glide.ry : py), max = axis === 'x' ? mx : my
      return d > 0 ? at < max - 0.5 : d < 0 ? at > 0.5 : false
    }
    for (let el: Element | null = target; el; el = el.parentElement ?? ((el.getRootNode() as ShadowRoot).host ?? null)) {
      if (root(el)) continue
      const style = view(el).getComputedStyle(el)
      // Native scrolling owns signed or reversed ranges (RTL, vertical writing, reversed flex).
      if (style.direction === 'rtl' || style.writingMode !== 'horizontal-tb' || style.flexDirection.endsWith('reverse')) { this.cancel(el); return null }
      const sx = scrollable(style.overflowX) && el.scrollWidth > el.clientWidth, sy = scrollable(style.overflowY) && el.scrollHeight > el.clientHeight
      if (!sx && !sy) continue
      if (style.scrollSnapType !== 'none' || style.overscrollBehavior !== 'auto auto' && style.overscrollBehavior !== 'auto') return null
      if (sx && can(el, 'x', dx) || sy && can(el, 'y', dy)) return dx && !sx || dy && !sy ? null : el
    }
    const doc = target.ownerDocument, top = doc.scrollingElement
    if (!top) return null
    const html = view(top).getComputedStyle(doc.documentElement), body = doc.body ? view(top).getComputedStyle(doc.body) : null
    if (html.direction === 'rtl' || html.writingMode !== 'horizontal-tb') { this.cancel(top); return null }
    const overflow = (axis: 'overflowX' | 'overflowY'): string => html[axis] !== 'visible' || !body ? html[axis] : body[axis]
    if (html.scrollSnapType !== 'none' || ['hidden', 'clip'].includes(overflow('overflowX')) && dx || ['hidden', 'clip'].includes(overflow('overflowY')) && dy) return null
    return can(top, 'x', dx) || can(top, 'y', dy) ? top : null
  }
}
