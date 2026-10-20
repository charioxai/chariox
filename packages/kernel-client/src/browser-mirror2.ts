import { Mirror2FontStyle, mirror2RenderedFontReady, mirror2DocumentFontsReady } from './browser-mirror2-fonts.js'
import { Mirror2SheetCache } from './browser-mirror2-sheets.js'
import { rememberMirror2Range, restoreMirror2Range } from './browser-mirror2-selection.js'
// MP-08/MP-10/MP-11: DOM mirror v2 renderer (local protocol 517). The page's own
// sanitized stylesheets and attributes are rebuilt in a script-free sandbox;
// deltas apply in place. No origin I/O: resources arrive as kernel bytes.
import { validateMirror2Packet, decodeMirror2Packet, resolveMirror2Sheets, mirror2SandboxCsp, validMirror2Sprite } from './browser-mirror2-security.js'
import { Mirror2Wheel } from './browser-mirror2-wheel.js'
import { Mirror2Navigation } from './browser-mirror2-navigation.js'
import type { Mirror2Action, Mirror2Form, Mirror2Op, Mirror2Packet, Mirror2Record, Mirror2Resource, Mirror2State, Mirror2Tile, Mirror2WirePacket } from './browser-mirror2-types.js'
export * from './browser-mirror2-types.js'
export { mirror2SandboxCsp, validateMirror2Packet, decodeMirror2Packet } from './browser-mirror2-security.js'
/** MP-08/MP-10/MP-11: protocol 550 versions the v2 wire (preview, fonts, sheets, flush, completion). */
export const browserMirror2MinimumProtocolVersion = 550
const NS = { svg: 'http://www.w3.org/2000/svg', math: 'http://www.w3.org/1998/Math/MathML' } as const
const resourcePattern = /url\("mr:(r[0-9]{1,9})"\)/g
const CREDIT_WINDOW = 4 // credits in flight at most (attachBrowserMirror2)
const IMAGE_HOLD_MS = 3000 // resource-quiet time after which a missing visible image stops holding the handover
const TEXT_LIMIT = 16384 // UTF-8 bytes per text action, matching the Rust boundary
const DELETIONS: Record<string, string[]> = {
  deleteContentBackward: ['Backspace'], deleteContentForward: ['Delete'], deleteWordBackward: ['Ctrl+Backspace'], deleteWordForward: ['Ctrl+Delete'],
  deleteSoftLineBackward: ['Shift+Home', 'Backspace'], deleteHardLineBackward: ['Shift+Home', 'Backspace'], deleteSoftLineForward: ['Shift+End', 'Delete'], deleteHardLineForward: ['Shift+End', 'Delete'],
}
// Cloud MP-08/MP-10: undo/redo go to the kernel's editor as its own (Linux Chromium) chords, Control=2 and Shift=8,
// as named mirror keys from protocol 530 (before: the document-bound display_input key with 517 modifiers).
type Chord = { kind: 'key'; key: string; modifiers: number }
type Mirror2Input = Mirror2Action | Chord
const HISTORY: Record<string, Chord> = { historyUndo: { kind: 'key', key: 'z', modifiers: 2 }, historyRedo: { kind: 'key', key: 'z', modifiers: 10 } }
// Controls whose state the viewer's own UI commits (its popup or picker); protocol 492 `set` carries the result.
const PICKED = new Set('date time datetime-local month week color range'.split(' '))
const picked = (el: Element): boolean => el.localName === 'select' || el.closest('select') !== null || el.localName === 'input' && PICKED.has((el as HTMLInputElement).type)
type Styled = { kind: 'css' | 'attr'; raw: string; node: Element; keys: string[]; fontStyle?: Mirror2FontStyle } | { kind: 'adopted'; raw: string[]; node: Document | ShadowRoot; keys: string[] }

function bytesOf(data: string): Uint8Array { return Uint8Array.from(atob(data), c => c.charCodeAt(0)) }
async function digest(bytes: Uint8Array): Promise<string> {
  return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes as Uint8Array<ArrayBuffer>)), b => b.toString(16).padStart(2, '0')).join('')
}

export type Mirror2Timing = { stage: string; duration_ms: number; ended_ms: number }

export class BrowserMirror2Renderer {
  private navigation: Mirror2Navigation
  get frame(): HTMLIFrameElement { return this.navigation.frame }
  get painted(): boolean { return this.navigation.painted }
  // Input is dropped while the interactive page is hidden, applying a packet or not yet painted.
  get acceptsInput(): boolean { return this.interactive && !this.applying && !this.disposed && !!this.documentId && this.navigation.painted }
  readonly timings: Mirror2Timing[] = []
  onTiming: ((sample: Mirror2Timing) => void) | undefined
  private doc: Document | null = null
  private navigationAt = performance.now()
  private paintFrame: HTMLIFrameElement | undefined
  private interactive = true
  private disposed = false
  private applying = false
  private dom = new Map<string, Node>()
  private records = new Map<string, Mirror2Record>()
  private ids = new WeakMap<Node, string>()
  private resources = new Map<string, string>()
  private sprites = new Map<string, string>() // protocol 501: validated sprite files as blob URLs ('' when refused)
  private slices = new Map<string, Mirror2Resource>() // resources still arriving in slices
  private styled = new Map<string, Styled>()
  private tileUrls = new Map<string, string>()
  private tileBoxes = new Map<string, Pick<Mirror2Tile, "x" | "y" | "width" | "height">>()
  private selects: Array<[Element, Mirror2Form]> = []
  private states: Element[] = [] // UA state to apply once this packet's DOM is attached
  private targeted = new WeakMap<Document, Element>() // per document, the element its fragment names (`:target`)
  private frameDocuments = new Set<string>() // frame document records (protocol 503: each has its own fragment)
  private bound = new WeakSet<Document>() // a retired frame document is not kept alive by its binding
  private empty: string
  sequence = 0
  documentId = ''
  private counts = { tile: 0, mask: 0 } // public diagnostics on the frame element (data-mirror-*)
  private pendingInputs = 0
  private focusSent: string | null = null // the focus the kernel has (applied from it, or sent to it)
  private selectionSent = ''
  private selectionHold: { key: string; until: number; epoch: number } | null = null
  private selectionFailure = false
  private pointerDown = false // a held viewer gesture owns its range, even between input receipts
  private releasePointer = (): void => { this.pointerDown = false }
  private pointerFocus: string | null = null // a focus taken by a pointer press: its click focuses in the kernel
  // After the kernel acknowledged a viewer focus, packets already in flight (one per credit) may have been drained
  // before it applied: those still naming the old kernel focus leave the viewer's alone (until the window passes).
  private focusHold: { id: string, previous: string | null, until: number } | null = null
  private kernelFocus: string | null = null // the latest packet's focus
  private focusWithheld: string | null | undefined // a packet's focus not taken while inputs were pending
  private resetSerial = 0 // local only: IDs and cached focus never cross a snapshot
  private queue: Array<{ resetSerial: number; action: Mirror2Input; epoch: { sequence: number; document_id: string } }> = []
  private sending = false
  private inputReplies = new Set<Promise<void>>() // MP-10: at most two written, unacknowledged inputs
  // Viewer-owned scroll (plan 4.1): kernel echoes wait until the viewer settles.
  private localScrollAt = -Infinity
  private localRootScrollAt = -Infinity
  private rootScrollReplies = new Set<{epoch:{document_id:string}}>()
  private applied = new WeakMap<Node, [number, number]>() // kernel-applied positions (their scroll events are not viewer input)
  private deferred = new Map<Node, [number, number]>() // latest kernel position per scroller that arrived while the viewer scrolled
  private wheel = new Mirror2Wheel(fn => requestAnimationFrame(fn), handle => cancelAnimationFrame(handle))
  // MP-08/MP-10/MP-11: a completed empty page can paint without another packet.
  onEmptyPaint: (() => void) | undefined
  constructor(private container: HTMLElement, private send: (action: Mirror2Input, epoch: { sequence: number; document_id: string }, onWritten?: () => void) => Promise<unknown>, _failure?: (error: unknown) => void) {
    this.empty = URL.createObjectURL(new Blob([]))
    this.navigation = new Mirror2Navigation(container, () => { if (!this.disposed) { this.navigationPaint(); this.onEmptyPaint?.() } })
    const owner = container.ownerDocument
    owner.addEventListener('pointerup', this.releasePointer, true)
    owner.addEventListener('pointercancel', this.releasePointer, true)
    owner.defaultView?.addEventListener('blur', this.releasePointer)
  }
  async ready(): Promise<void> {
    await this.navigation.loaded
    if (this.disposed) throw Error('MP-08: mirror closed')
    this.doc = this.frame.contentDocument
    if (!this.doc) throw Error('MP-11: mirror sandbox unavailable')
    this.bind(this.doc)
  }
  private timed(stage: string, at: number): void {
    const now = performance.now(), sample = { stage, duration_ms: now - at, ended_ms: performance.timeOrigin + now }
    this.timings.push(sample); if (this.timings.length > 2048) this.timings.splice(0, 1024)
    try { this.onTiming?.(sample) } catch { /* diagnostic observers cannot interrupt paint */ }
  }
  // MP-08/MP-10/MP-11: first visible navigation paint, after a paint opportunity.
  // Keep identity local; diagnostics contain only the fixed stage and durations.
  private navigationPaint(): void {
    const frame = this.frame
    if (!this.painted || this.paintFrame === frame || typeof requestAnimationFrame !== 'function') return
    this.paintFrame = frame
    const started = this.navigationAt
    requestAnimationFrame(() => requestAnimationFrame(() => {
      if (!this.disposed && this.frame === frame && this.painted) this.timed('mirror_navigation_paint', started)
    }))
  }
  // ---- input: node-addressed, the kernel re-validates everything live.
  private enqueue(action: Mirror2Input): void {
    if (!this.acceptsInput) return
    const epoch = { sequence: this.sequence, document_id: this.documentId }
    // Ordered; a queued scroll position for the same scroller is replaced, not appended.
    const last = this.queue.at(-1)
    if (action.kind === 'scroll_to' && last?.action.kind === 'scroll_to' && last.action.node_id === action.node_id) { last.action = action; last.epoch = epoch; last.resetSerial = this.resetSerial; return }
    if (action.kind === 'selection' && this.records.get(action.anchor_id)?.kind === 'text') { this.selectionSent = JSON.stringify(action); this.selectionHold = { key: this.selectionSent, until: Infinity, epoch: epoch.sequence }; this.selectionFailure = false }
    this.queue.push({ action, epoch, resetSerial: this.resetSerial }); this.pendingInputs++
    void this.pump()
  }
  // Rust bounds UTF-8 bytes; never split a code point across ordered actions.
  private enqueueText(text: string): void {
    let at = 0, end = 0, bytes = 0
    for (const point of text) {
      const code = point.codePointAt(0)!
      const size = code <= 0x7f ? 1 : code <= 0x7ff ? 2 : code <= 0xffff ? 3 : 4
      if (bytes + size > TEXT_LIMIT) { this.enqueue({ kind: 'text', text: text.slice(at, end) }); at = end; bytes = 0 }
      end += point.length; bytes += size
    }
    if (end > at) this.enqueue({ kind: 'text', text: text.slice(at, end) })
  }
  private async pump(): Promise<void> {
    if (this.sending) return
    this.sending = true
    try {
      while (this.queue.length && !this.disposed) {
        if (this.inputReplies.size >= 2) await Promise.race(this.inputReplies)
        const next = this.queue.shift(); if (!next || this.disposed) break
        const before = this.kernelFocus
        if (next.action.kind === 'scroll_to' && next.action.node_id === null) this.rootScrollReplies.add(next)
        let written = () => {}
        const sent = new Promise<void>(resolve => { written = resolve })
        // MP-08/MP-10/MP-11: preserve actual write order, including asynchronous
        // encryption/admission. The kernel has ordered this viewer's input since
        // protocol516 (v2 requires517). Legacy transports release only on reply.
        const replied:Promise<void> = Promise.resolve().then(() => {
          if (!this.disposed) return this.send(next.action, next.epoch, written)
        }).then(() => { if (!this.disposed && next.resetSerial === this.resetSerial) { this.holdFocus(next.action, before); if (next.action.kind === 'selection' && this.selectionHold?.key === JSON.stringify(next.action) && this.selectionHold.epoch === next.epoch.sequence && next.epoch.document_id === this.documentId) this.selectionHold.until = this.sequence + CREDIT_WINDOW } })
          .catch(() => { if (next.resetSerial === this.resetSerial && next.action.kind === 'selection' && this.selectionHold?.key === JSON.stringify(next.action) && this.selectionHold.epoch === next.epoch.sequence && next.epoch.document_id === this.documentId) { this.selectionFailure = true; this.selectionHold = null; this.selectionSent = '' } if (!this.disposed) this.frame.dataset.mirrorRefusals = String(Number(this.frame.dataset.mirrorRefusals ?? 0) + 1) })
          .finally(() => {
            this.inputReplies.delete(replied); this.rootScrollReplies.delete(next); this.pendingInputs = Math.max(0, this.pendingInputs - 1)
            written(); this.settleFocus()
          })
        this.inputReplies.add(replied)
        await sent
      }
    } finally { this.sending = false; this.settleFocus() }
  }
  // MP-08/MP-10/MP-11: Notes captures the kernel's range after every released
  // selection input has replied, including work still waiting in the write queue.
  async settleInput(): Promise<void> {
    const documentId=this.documentId
    // selectionchange is asynchronous: Notes can be clicked before the final
    // release endpoint has emitted its event. Sample it before awaiting receipts.
    this.syncSelection(this.selectionDoc ?? this.doc)
    while (this.pendingInputs && !this.disposed) await Promise.all([...this.inputReplies])
    if (this.disposed) throw Error('MP-08: mirror closed')
    if(this.documentId!==documentId)throw Error('MP-08: mirror document changed')
    if (!this.selectedText(this.selectionDoc ?? this.doc)) this.selectionFailure = false
    if(this.selectionFailure)throw Error('MP-10: mirror selection refused')
  }
  private settleFocus(): void {
    if (this.pendingInputs || this.disposed || this.pointerDown) return
    const withheld = this.focusWithheld; this.focusWithheld = undefined
    if (withheld !== undefined && !this.pointerFocus && !this.focusHold) this.takeFocus(withheld)
  }
  private holdFocus(action: Mirror2Input, before: string | null): void {
    // A packet since the input was sent already moved the kernel's focus: it reflects the input, nothing to hold.
    if (!['focus', 'click', 'selection'].includes(action.kind) || this.kernelFocus !== before) return
    const active = this.active(), id = active && active !== active.ownerDocument.body && active !== active.ownerDocument.documentElement ? this.ids.get(active) : undefined
    if (id && id !== this.kernelFocus && this.records.get(id)?.kind === 'element') this.focusHold = { id, previous: this.kernelFocus, until: this.sequence + CREDIT_WINDOW }
  }
  // The kernel's focus in the viewer: the viewer page's own control outside the mirror keeps its focus.
  private takeFocus(id: string | null): void {
    if (!this.interactive) return
    const outside = this.frame.ownerDocument.activeElement, elsewhere = !!outside && outside !== this.frame && outside !== this.frame.ownerDocument.body
    const focused = id && !elsewhere ? this.dom.get(id) as HTMLElement | undefined : undefined, active = this.active() as HTMLElement | null
    this.focusSent = id
    // A control built while detached could not take its caret: restore it once focused.
    if (focused && active !== focused) { focused.focus?.({ preventScroll: true }); const form = this.records.get(id!)?.form; if (form) this.caret(focused as HTMLInputElement, form) }
    // The kernel's focus left (blur, a protected field): the viewer's control blurs too.
    else if (!id && !elsewhere && active && this.ids.has(active) && active !== active.ownerDocument.body && active !== active.ownerDocument.documentElement) active.blur()
  }
  // A text control's own range (select-all, a drag inside it) reaches the kernel
  // before the key or text that replaces it; ranges it already has are not resent.
  private ranges = new WeakMap<Element, string>()
  private rangeOf(field: HTMLInputElement): string { return `${field.selectionStart},${field.selectionEnd},${field.selectionDirection}` }
  private syncRange(send = true): void {
    const field = this.active() as HTMLInputElement | null, id = field ? this.ids.get(field) : undefined
    if (!field || !id || this.records.get(id)?.kind !== 'element' || !['input', 'textarea'].includes(field.localName) || field.selectionStart === null || field.selectionEnd === null || this.ranges.get(field) === this.rangeOf(field)) return
    this.ranges.set(field, this.rangeOf(field)); if (!send) return
    const [anchor, focus] = field.selectionDirection === 'backward' ? [field.selectionEnd, field.selectionStart] : [field.selectionStart, field.selectionEnd]
    this.enqueue({ kind: 'selection', anchor_id: id, anchor_offset: anchor, focus_id: id, focus_offset: focus })
  }
  private selectedText(doc: Document | null): Mirror2Input | null {
    const selection = doc?.getSelection()
    if (!selection || selection.isCollapsed || !selection.anchorNode?.isConnected || !selection.focusNode?.isConnected) return null
    const a = this.ids.get(selection.anchorNode), b = this.ids.get(selection.focusNode)
    if (!a || !b || this.records.get(a)?.kind !== 'text' || this.records.get(b)?.kind !== 'text') return null
    return { kind: 'selection', anchor_id: a, anchor_offset: selection.anchorOffset, focus_id: b, focus_offset: selection.focusOffset }
  }
  private syncSelection(doc: Document | null): void {
    const action = this.selectedText(doc)
    if (!action) { this.selectionFailure = false; return }
    this.selectionDoc = doc
    if (JSON.stringify(action) !== this.selectionSent) this.enqueue(action)
  }
  private scrolling(): boolean { return performance.now() - this.localScrollAt < 500 || this.queue.some(q => q.action.kind === 'scroll_to') }
  private rootScrolling(): boolean { return performance.now() - this.localRootScrollAt < 500 || [...this.rootScrollReplies].some(q => q.epoch.document_id === this.documentId) || this.queue.some(q => q.action.kind === 'scroll_to' && q.action.node_id === null && q.epoch.document_id === this.documentId) }
  private idOf(node: Node | null | undefined): string | undefined {
    for (let n: Node | null | undefined = node, depth = 0; n && depth < 512; depth++) { const id = this.ids.get(n); if (id) return id; n = n.parentNode ?? (n as ShadowRoot).host }
    return undefined
  }
  private bind(doc: Document): void {
    if (this.bound.has(doc)) return
    this.bound.add(doc)
    const on = (type: string, fn: (event: Event) => void): void => doc.addEventListener(type, event => {
      if (this.disposed || !this.navigation.painted) return
      // A held/retired iframe can retain native keyboard focus. Its event may
      // never acquire the new document's epoch, including root key actions.
      let frame = doc.defaultView?.frameElement
      for (let depth = 0; frame && depth < 512; depth++) {
        if (frame === this.frame) { fn(event); return }
        frame = frame.ownerDocument.defaultView?.frameElement
      }
    }, { capture: true, passive: false })
    const target = (event: Event): Element | null => { const node = event.composedPath()[0] as Node; return node?.nodeType === 1 ? node as Element : node?.parentElement ?? null }
    const element = (event: Event): { id: string; el: Element } | null => {
      for (let el = target(event); el; el = el.parentElement ?? ((el.getRootNode() as ShadowRoot).host ?? null)) { const id = this.ids.get(el); if (id && this.records.get(id)?.kind !== 'text') return this.records.get(id)?.kind === 'mask' ? null : { id, el } }
      return null
    }
    const offset = (el: Element, event: MouseEvent, r: DOMRect = el.getBoundingClientRect()): { x: number; y: number } => ({ x: event.clientX - r.left, y: event.clientY - r.top })
    // A click that ends a text drag selects; it is not forwarded as a click.
    // MP-08/MP-10: its target's box is read at the press, while layout is clean from the press's hit test; read at
    // the click it forces the style and layout the press dirtied (focus, :active; 5-6 ms on a large page) ahead of the input.
    let origin: { x: number; y: number } | null = null, dragged = false, pressed: { el: Element; box: DOMRect } | null = null
    on('pointerdown', event => { const e = event as PointerEvent; if (e.button === 0) this.pointerDown = true; origin = { x: e.clientX, y: e.clientY }; dragged = false; this.pointerFocus = null; const hit = element(event); pressed = hit ? { el: hit.el, box: hit.el.getBoundingClientRect() } : null })
    // A press whose gesture forwards no click (a drag, a select or picker, a release elsewhere) sends its focus now.
    on('pointerup', () => { this.syncSelection(doc); this.releasePointer(); origin = null; setTimeout(() => { const id = this.pointerFocus; this.pointerFocus = null; if (id && !this.disposed) { this.focusSent = id; this.enqueue({ kind: 'focus', node_id: id }) } }, 0) })
    on('pointercancel', () => { this.releasePointer(); origin = null; this.pointerFocus = null; pressed = null; dragged = false })
    on('pointermove', event => { const e = event as PointerEvent; if (origin && (e.buttons & 1) && Math.hypot(e.clientX - origin.x, e.clientY - origin.y) > 4) dragged = true })
    const selecting = (el: Element): boolean => { if (el.closest('input,textarea,button,select,[contenteditable]')) return false; const s = doc.getSelection(); return Boolean(s && !s.isCollapsed && (el.contains(s.anchorNode) || el.contains(s.focusNode))) }
    // A drag inside a text control focused it here only: its range goes now (the kernel focuses the control with it).
    // A select or picker opens the viewer's own UI; its committed state goes as `set` (the kernel's popup never opens).
    on('click', event => {
      if (pressed && !pressed.el.isConnected) { event.preventDefault(); pressed = null; dragged = false; this.pointerFocus = null; return }
      const hit = element(event); if (hit && picked(hit.el)) return
      event.preventDefault()
      if (!hit || dragged || selecting(hit.el)) { if (dragged) { const id = this.pointerFocus; this.pointerFocus = null; if (id) { this.focusSent = id; this.enqueue({ kind: 'focus', node_id: id }) } this.syncRange() } dragged = false; return }
      // The kernel's click focuses what the viewer's press focused: no separate round trip before it.
      const box = pressed?.el === hit.el ? pressed.box : undefined; pressed = null
      this.pointerFocus = null; this.enqueue({ kind: 'click', node_id: hit.id, ...offset(hit.el, event as MouseEvent, box) })
    })
    // Protocol 492: focus transitions and committed control state, not inferred from clicks or keys.
    on('focusin', event => { const id = this.ids.get(event.composedPath()[0] as Node); if (this.applying || !id || this.records.get(id)?.kind !== 'element' || id === this.focusSent) return; if (origin) { this.pointerFocus = id; return } this.focusSent = id; this.enqueue({ kind: 'focus', node_id: id }) })
    // Focus left to nothing (not to another control, not the viewer window losing focus): the kernel blurs too.
    on('focusout', event => {
      const node = event.composedPath()[0] as Element, id = this.ids.get(node)
      if (this.applying || !id || (event as FocusEvent).relatedTarget) return
      setTimeout(() => { const active = this.active(); if (this.disposed || this.applying || !this.doc?.hasFocus() || active === node || active && active !== active.ownerDocument.body && active !== active.ownerDocument.documentElement && active.localName !== 'iframe' || this.focusSent !== id) return; this.focusSent = null; this.enqueue({ kind: 'blur', node_id: id }) }, 0)
    })
    on('change', this.committed)
    on('auxclick', event => event.preventDefault())
    on('dragstart', event => event.preventDefault())
    on('submit', event => event.preventDefault())
    // Local scrolling, eased per frame (MP-10); opaque regions (canvas, video, foreign stills) take the wheel themselves.
    on('wheel', event => {
      const hit = element(event), wheel = event as WheelEvent
      if (!hit || this.records.get(hit.id)?.kind !== 'tile') {
        const view = doc.defaultView!, x = view.scrollX, y = view.scrollY
        if (this.wheel.take(wheel, target(event))) event.preventDefault()
        this.localScrollAt = performance.now()
        if (doc === this.doc && (this.wheel.rootTarget(doc) || view.scrollX !== x || view.scrollY !== y)) this.localRootScrollAt = this.localScrollAt
        return
      }
      event.preventDefault(); this.enqueue({ kind: 'scroll', node_id: hit.id, ...offset(hit.el, wheel), delta_x: Math.trunc(wheel.deltaX), delta_y: Math.trunc(wheel.deltaY) })
    })
    on('scroll', this.scrolled)
    on('keydown', event => {
      const { key, shiftKey, ctrlKey, metaKey, altKey, isComposing, keyCode } = event as KeyboardEvent
      // Keys while composing belong to the viewer's IME (candidates, confirmation); its range is
      // not the kernel's either. The committed text travels on compositionend.
      if (isComposing || keyCode === 229) return
      this.syncRange()
      if (ctrlKey || metaKey || altKey) {
        const history = altKey ? undefined : /^z$/i.test(key) ? HISTORY[shiftKey ? 'historyRedo' : 'historyUndo'] : ctrlKey && !shiftKey && /^y$/i.test(key) ? HISTORY.historyRedo : undefined
        if (history) { event.preventDefault(); this.enqueue(history) }
        return
      }
      // A select or picker keeps its keys (typeahead, arrows, its popup): the committed state goes as `set`.
      const active = this.active(); if (active && picked(active) && !['Tab', 'Enter'].includes(key)) return
      // Page keys scroll the viewer's own copy natively; the scroll listener sends the position.
      // Ordinary physical keys use the existing native key path. Page keydown
      // handlers (including deferred search widgets) run before insertion.
      // Paste and IME keep beforeinput/composition's committed text path.
      if (/^[^\p{C}]$/u.test(key)) { event.preventDefault(); this.enqueue({ kind: 'key', key }); return }
      if (['Tab', 'Enter', 'Escape', 'Backspace', 'Delete', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(key)) {
        event.preventDefault()
        // Shift keeps its meaning for focus and selection keys (back-tab, extending a range).
        this.enqueue({ kind: 'key', key: shiftKey && ['Tab', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(key) ? `Shift+${key}` : key })
      }
    })
    let composed: string | null = null
    // Deletions the key handler leaves native (modified Backspace/Delete, cut) travel as the
    // kernel's editing keys; a range (synced first) goes as a whole.
    const ranged = (): boolean => { const field = this.active() as HTMLInputElement | null; return typeof field?.selectionStart === 'number' ? field.selectionStart !== field.selectionEnd : doc.getSelection()?.isCollapsed === false }
    on('beforeinput', event => {
      const input = event as InputEvent; event.preventDefault()
      if (input.isComposing || input.inputType.includes('Composition')) return
      const history = HISTORY[input.inputType]; if (history) { this.syncRange(); this.enqueue(history); return }
      if (input.inputType.startsWith('delete')) { this.syncRange(); for (const key of ranged() ? ['Delete'] : DELETIONS[input.inputType] ?? []) this.enqueue({ kind: 'key', key }); return }
      const data = input.data ?? (input.inputType.startsWith('insertFromPaste') ? input.dataTransfer?.getData('text/plain') : null)
      if (data === composed || !data) return
      this.syncRange(); this.enqueueText(data)
    })
    on('compositionend', event => { event.preventDefault(); const data = (event as CompositionEvent).data; if (data) { composed = data; setTimeout(() => { composed = null }, 0); this.enqueueText(data) } this.syncRange(false) })
    // The range a composition replaces is synced as it starts; its result is where the kernel's caret ends.
    on('compositionstart', () => this.syncRange())
    on('selectionchange', () => { if (!this.applying) this.syncSelection(doc) })
  }
  // A multiple select's option state last sent per option, until the kernel's record for it changes: two opposing
  // changes before the first echo compare with what was sent, not with the unchanged kernel record.
  private optionSent = new WeakMap<Element, boolean>()
  // Committed control state (protocol 492 `set`). Bound on documents and shadow roots: `change` is not composed.
  private committed = (event: Event): void => {
    const control = event.composedPath()[0] as HTMLSelectElement & HTMLInputElement, id = this.ids.get(control)
    if (this.applying || !id || this.records.get(id)?.kind !== 'element' || !picked(control)) return
    if (control.localName === 'input') this.enqueue({ kind: 'set', node_id: id, value: control.value })
    else if (!control.multiple) this.enqueue({ kind: 'set', node_id: id, selected_index: control.selectedIndex })
    else for (const option of Array.from(control.options)) {
      const option_id = this.ids.get(option), state = option_id ? this.records.get(option_id)?.form : undefined
      if (option_id && state && (this.optionSent.get(option) ?? state.checked) !== option.selected) { this.optionSent.set(option, option.selected); this.enqueue({ kind: 'set', node_id: option_id, checked: option.selected }) }
    }
  }
  // Viewer scrolls (documents and shadow roots: element scroll events do not cross a shadow boundary).
  private scrollTargets = new Set<Node>()
  private scrollFrame = 0
  private scrolled = (event: Event): void => {
    const target = event.target as Node, kernel = this.applied.get(target)
    const now = target.nodeType === 9 ? [(target as Document).defaultView?.scrollX ?? 0, (target as Document).defaultView?.scrollY ?? 0] : [(target as Element).scrollLeft, (target as Element).scrollTop]
    if (this.applying || kernel && Math.abs(kernel[0] - now[0]!) < 1 && Math.abs(kernel[1] - now[1]!) < 1) return
    this.applied.delete(target)
    // The viewer's own scroll is the last writer of this scroller.
    this.deferred.delete(target); if (target.nodeType === 9) this.deferred.delete((target as Document).documentElement)
    this.localScrollAt = performance.now(); this.scrollTargets.add(target)
    if (target === this.doc) this.localRootScrollAt = this.localScrollAt
    this.scrollFrame ||= requestAnimationFrame(() => {
      this.scrollFrame = 0; const targets = this.scrollTargets; this.scrollTargets = new Set()
      for (const target of targets) {
        const doc = target.nodeType === 9 ? target as Document : null, id = this.ids.get(target)
        if (doc) this.enqueue({ kind: 'scroll_to', node_id: doc === this.doc ? null : id ?? null, x: doc.defaultView?.scrollX ?? 0, y: doc.defaultView?.scrollY ?? 0 })
        else if (id) this.enqueue({ kind: 'scroll_to', node_id: id, x: (target as Element).scrollLeft, y: (target as Element).scrollTop })
      }
    })
  }
  // ---- resources
  private substitute(raw: string): string { return raw.replace(resourcePattern, (_, key: string) => `url("${this.resources.get(key) ?? this.empty}")`) }
  private keysOf(raw: string): string[] { return [...raw.matchAll(resourcePattern)].map(m => m[1]!) }
  private async addResource(resource: Mirror2Resource): Promise<void> {
    const bytes = bytesOf(resource.data_base64)
    if (await digest(bytes) !== resource.resource_id) throw Error('MP-11: mirror resource digest')
    const old = this.resources.get(resource.key); if (old?.startsWith('blob:')) URL.revokeObjectURL(old)
    const sprite = this.sprites.get(resource.key); if (sprite) URL.revokeObjectURL(sprite); this.sprites.delete(resource.key)
    // SVG stays an image: data: URLs have an opaque origin if ever opened top-level.
    this.resourceAt = performance.now()
    this.resources.set(resource.key, resource.mime_type === 'image/svg+xml' ? `data:image/svg+xml;base64,${resource.data_base64}` : URL.createObjectURL(new Blob([bytes as Uint8Array<ArrayBuffer>], { type: resource.mime_type })))
  }
  // Slices of one resource arrive in order; the digest covers the whole.
  private assemble(resource: Mirror2Resource): Mirror2Resource | null {
    if (resource.total === undefined) return resource
    const partial = resource.offset === 0 ? { ...resource, data_base64: '' } : this.slices.get(resource.key)
    if (!partial || partial.data_base64.length !== resource.offset || partial.resource_id !== resource.resource_id || partial.mime_type !== resource.mime_type || partial.total !== resource.total) throw Error('MP-08: mirror resource slice out of order')
    partial.data_base64 += resource.data_base64
    if (partial.data_base64.length < resource.total) { this.slices.set(resource.key, partial); return null }
    this.slices.delete(resource.key)
    return { key: resource.key, resource_id: resource.resource_id, mime_type: resource.mime_type, data_base64: partial.data_base64 }
  }
  private restyle(entry: Styled): void {
    if (entry.kind === 'css') { if (entry.fontStyle) entry.fontStyle.paint(text => this.substitute(text)); else entry.node.textContent = this.substitute(entry.raw) }
    else if (entry.kind === 'attr') { entry.node.setAttribute('style', this.substitute(entry.raw)); const record = this.records.get(this.ids.get(entry.node) ?? ''); if (record) this.opaqueLayout(record, entry.node) }
    else {
      const { node: root, raw } = entry as Extract<Styled, { kind: 'adopted' }>
      const view = (root.nodeType === 9 ? root as Document : (root as ShadowRoot).ownerDocument).defaultView as (Window & typeof globalThis) | null
      if (view) this.adopt(root, raw.map(text => { const sheet = new view.CSSStyleSheet(); sheet.replaceSync(this.substitute(text)); return sheet }))
    }
  }
  // Author sheets of a document, led by its font shadow: for each stack name the native machine lacks, a face that covers no
  // character and is never fetched (a failed face would hand the name back to the system fonts), so the stack falls
  // through to the next family here as it did natively.
  private shadows = new WeakMap<Document, CSSStyleSheet>()
  private adopt(root: Document | ShadowRoot, sheets: CSSStyleSheet[]): void {
    const shadow = root.nodeType === 9 ? this.shadows.get(root as Document) : undefined
    root.adoptedStyleSheets = shadow ? [shadow, ...sheets] : sheets
  }
  private shadowFonts(doc: Document, names: readonly string[]): void {
    const held = this.shadows.get(doc), view = doc.defaultView as (Window & typeof globalThis) | null
    this.shadows.delete(doc)
    if (names.length && view?.CSSStyleSheet) {
      const sheet = new view.CSSStyleSheet()
      sheet.replaceSync(names.map(name => `@font-face{font-family:${JSON.stringify(name)};src:url(${this.empty});unicode-range:U+10FFFF}`).join(''))
      this.shadows.set(doc, sheet)
    }
    this.adopt(doc, doc.adoptedStyleSheets.filter(sheet => sheet !== held))
  }
  private setStyled(id: string, entry: Styled | null): void {
    const previous = this.styled.get(id)
    if (!entry) { if (previous?.kind === 'css') previous.fontStyle?.remove(); this.styled.delete(id); return }
    if (entry.kind === 'css') { entry.fontStyle = (previous?.kind === 'css' && previous.node === entry.node ? previous.fontStyle : undefined) ?? new Mirror2FontStyle(entry.node); entry.fontStyle.set(entry.raw) }
    entry.keys = entry.kind === 'adopted' ? entry.raw.flatMap(text => this.keysOf(text)) : this.keysOf(entry.raw)
    this.styled.set(entry.kind === 'adopted' ? `${id}#adopted` : entry.kind === 'attr' ? `${id}#style` : id, entry)
    this.restyle(entry)
  }
  // ---- DOM construction
  private create(record: Mirror2Record, doc: Document): Node {
    // Replacement records reuse source IDs. Retire the old index before publishing
    // the new node, so removing the old subtree cannot forget its replacement.
    if (this.dom.has(record.id)) this.forget(record.id)
    let node: Node
    if (record.kind === 'text') node = doc.createTextNode(record.text ?? '')
    else if (record.kind === 'document' || record.kind === 'shadow') node = doc.createDocumentFragment()
    else node = record.ns ? doc.createElementNS(NS[record.ns], record.tag!) : doc.createElement(record.tag ?? 'span')
    this.dom.set(record.id, node); this.ids.set(node, record.id); this.records.set(record.id, record)
    if (record.kind === 'tile' || record.kind === 'mask') this.counts[record.kind]++
    if (node.nodeType === 1) this.decorate(record, node as Element)
    return node
  }
  private decorate(record: Mirror2Record, element: Element): void {
    for (const [name, value] of Object.entries(record.attrs ?? {})) {
      if (name === 'style') this.setStyled(record.id, { kind: 'attr', raw: value, node: element, keys: [] })
      // The kernel opens and closes popovers (state ops): the viewer's copy never light-dismisses itself.
      else try { element.setAttribute(name, name === 'popover' ? 'manual' : value) } catch { /* invalid names were refused by validation */ }
    }
    if (record.state) this.states.push(element)
    if (record.kind === 'element' && record.tag === 'style') this.setStyled(record.id, { kind: 'css', raw: record.css ?? '', node: element, keys: [] })
    if (record.res !== undefined) this.image(record.id, element, record.res)
    this.opaqueLayout(record, element)
    if (record.kind === 'frame') { element.setAttribute('sandbox', 'allow-same-origin'); element.setAttribute('referrerpolicy', 'no-referrer') }
    // A select's state waits for its options (the build appends children after their parent).
    if (record.form) { if (element.localName === 'select') this.selects.push([element, record.form]); else this.form(element, record.form) }
  }
  // Author layout remains ordinary CSS; capture paint overrides are restored separately.
  private opaqueLayout(record: Mirror2Record, element: Element): void {
    if (record.kind === 'mask' || record.kind === 'tile') {
      const style = (element as HTMLElement).style
      const [width, height] = record.size ?? [0, 0]
      style.setProperty('box-sizing', 'border-box', 'important'); style.setProperty('width', `${width}px`, 'important'); style.setProperty('height', `${height}px`, 'important')
      style.setProperty('overflow', 'hidden', 'important'); style.setProperty('background-color', 'black', 'important'); style.setProperty('color', 'transparent', 'important')
      // A mask keeps the protected box: inline/contents boxes become inline blocks of the source size.
      if (record.kind === 'mask') { style.setProperty('display', record.display === 'none' ? 'none' : !record.display || ['inline', 'contents'].includes(record.display) ? 'inline-block' : record.display, 'important'); style.setProperty('background-image', 'none', 'important'); element.setAttribute('aria-label', 'Protected content') }
      if (record.kind === 'tile') { if (element.localName === 'iframe') element.setAttribute('sandbox', ''); this.paintTile(record.id, element as HTMLElement) }
    }
  }
  private uaState(element: Element): void {
    const record = this.records.get(this.ids.get(element) ?? ''), state: Mirror2State = record?.state ?? {}
    if (!record || !element.isConnected) return
    const html = element as HTMLElement
    // `:defined` is rewritten to :not([chariox-undefined]) by the kernel's sanitizer: no viewer registry
    // definition outlives its source document (a reset or replaced frame document reuses the viewer's).
    if (element.localName.includes('-')) element.toggleAttribute('chariox-undefined', state.defined === false)
    try {
      if (element.hasAttribute('popover') && html.matches(':popover-open') !== Boolean(state.popover_open)) { if (state.popover_open) html.showPopover(); else html.hidePopover() }
      if (element.localName === 'dialog') {
        const dialog = element as HTMLDialogElement
        if (state.modal && !dialog.matches(':modal')) { if (dialog.open) dialog.close(); dialog.showModal() }
        else if (!state.modal && dialog.matches(':modal')) { dialog.close(); if (record.attrs?.open !== undefined) dialog.show() }
      }
      if (element.localName === 'input') (element as HTMLInputElement).indeterminate = Boolean(state.indeterminate)
      // Protocol 501: the kernel's custom validity, without its message (the viewer never reports it).
      if ('setCustomValidity' in element) (element as HTMLInputElement).setCustomValidity(state.custom_invalid ? 'invalid' : '')
    } catch { /* a state the viewer cannot take stays at its default */ }
  }
  // `:target` (rewritten to [chariox-target] by the kernel's sanitizer) marks the element a document's fragment
  // names: the top document's from the packet header, a frame document's from its record (protocol 503).
  private target(doc: Document, fragment: string | null): void {
    let next: Element | null = null
    if (fragment) { let name = fragment; try { name = decodeURIComponent(fragment) } catch { /* raw */ } next = doc.getElementById(name) ?? Array.from(doc.getElementsByName(name)).find(e => e.localName === 'a') ?? null }
    const previous = this.targeted.get(doc) ?? null
    if (next === previous && next?.hasAttribute('chariox-target') !== false) return
    previous?.removeAttribute('chariox-target'); next?.setAttribute('chariox-target', '')
    if (next) this.targeted.set(doc, next); else this.targeted.delete(doc)
  }
  private targets(fragment: string | null): void {
    if (this.doc) this.target(this.doc, fragment)
    for (const id of this.frameDocuments) { const doc = this.dom.get(id), record = this.records.get(id); if (doc?.nodeType === 9 && record) this.target(doc as Document, record.fragment ?? null); else this.frameDocuments.delete(id) }
  }
  // Protocol 501: a <use> needs a same-origin document (data: URLs are not followed), validated here first.
  private sprite(key: string): string | null {
    let url = this.sprites.get(key)
    if (url === undefined) {
      const data = this.resources.get(key); if (!data?.startsWith('data:image/svg+xml;base64,')) return null
      const text = new TextDecoder().decode(bytesOf(data.slice(26)))
      url = validMirror2Sprite(text, this.frame.ownerDocument.defaultView as Window & typeof globalThis) ? URL.createObjectURL(new Blob([text], { type: 'image/svg+xml' })) : ''
      this.sprites.set(key, url)
    }
    return url || null
  }
  private image(id: string, element: Element, key: string | null): void {
    const url = key ? this.resources.get(key) ?? null : null
    if (element.localName === 'img') { if (url) element.setAttribute('src', url); else element.removeAttribute('src') }
    else if (element.localName === 'use') { const file = url && key ? this.sprite(key) : null; if (file) element.setAttribute('href', `${file}${this.records.get(id)?.attrs?.href ?? ''}`); else element.removeAttribute('href') }
    else if (element.namespaceURI === NS.svg) { if (url) element.setAttribute('href', url); else element.removeAttribute('href') }
    const record = this.records.get(id); if (record) { if (key) record.res = key; else delete record.res }
  }
  private form(element: Element, form: Mirror2Form): void {
    // An option of a multiple select carries its own selectedness (`checked`).
    if (element.localName === 'option') { (element as HTMLOptionElement).selected = form.checked; this.optionSent.delete(element); return }
    const field = element as HTMLInputElement
    if (field.value !== form.value) field.value = form.value
    if ('checked' in field && field.type !== undefined) field.checked = form.checked
    if (element.localName === 'select') {
      const select = element as unknown as HTMLSelectElement
      // A selectedIndex write would keep only the first of a multiple selection.
      if (!select.multiple) select.selectedIndex = form.selected_index
      else for (const option of Array.from(select.options)) { const state = this.records.get(this.ids.get(option) ?? '')?.form; if (state) option.selected = state.checked }
    }
    if ((field.getRootNode() as Document | ShadowRoot).activeElement === field) this.caret(field, form)
  }
  private caret(field: HTMLInputElement, form: Mirror2Form): void {
    if (form.selection_start !== null && form.selection_end !== null) try { field.setSelectionRange(form.selection_start, form.selection_end); this.ranges.set(field, this.rangeOf(field)) } catch { /* not a text control */ }
  }
  private selectionDoc: Document | null = null // the document showing the kernel's selection
  private editing(doc: Document): boolean {
    let element = doc.activeElement
    for (let depth = 0; depth < 64 && element?.shadowRoot?.activeElement; depth++) element = element.shadowRoot.activeElement
    return ['input', 'textarea'].includes(element?.localName ?? '')
  }
  // The focused leaf across open shadow roots and mirrored frame documents.
  private active(): Element | null {
    let element = this.doc?.activeElement ?? null
    for (let depth = 0; depth < 64 && element; depth++) { const next = element.shadowRoot?.activeElement ?? (element.localName === 'iframe' ? (element as HTMLIFrameElement).contentDocument?.activeElement : null); if (!next || next === element) break; element = next }
    return element
  }
  // Records are pre-order; each record's children follow it in list order.
  // Frame documents are built separately into the frame's own document.
  private build(records: Mirror2Record[], doc: Document, frames: Array<{ frame: HTMLIFrameElement; id: string }>, scrolls: Array<[Element, number, number]>): Map<string, Node> {
    const built = new Map<string, Node>(), shadows: Array<[Element, Mirror2Record]> = [], skipped = new Set<string>(), outer = this.selects
    this.selects = []
    for (const record of records) {
      if (record.parent !== null && (skipped.has(record.parent) || this.records.get(record.parent)?.kind === 'frame')) { skipped.add(record.id); continue }
      const node = this.create(record, doc); built.set(record.id, node)
      if (record.scroll && node.nodeType === 1) scrolls.push([node as Element, record.scroll[0], record.scroll[1]])
      if (record.kind === 'frame') frames.push({ frame: node as HTMLIFrameElement, id: record.id })
      const parent = record.parent !== null ? built.get(record.parent) : undefined
      if (record.kind === 'shadow') { if (parent) shadows.push([parent as Element, record]); continue }
      parent?.appendChild(node)
    }
    for (const [host, record] of shadows) this.attachShadow(host, record, built.get(record.id) as DocumentFragment)
    for (const [element, form] of this.selects) this.form(element, form)
    this.selects = outer
    return built
  }
  private attachShadow(host: Element, record: Mirror2Record, fragment: DocumentFragment): void {
    let root: ShadowRoot | null = host.shadowRoot
    if (!root) try { root = host.attachShadow({ mode: 'open' }); root.addEventListener('scroll', this.scrolled, { capture: true, passive: true }); root.addEventListener('change', this.committed, { capture: true }) } catch { root = null }
    if (!root) { this.forget(record.id); return }
    root.replaceChildren(...Array.from(fragment.childNodes))
    this.dom.set(record.id, root); this.ids.set(root, record.id)
    if (record.adopted?.length) this.setStyled(record.id, { kind: 'adopted', raw: record.adopted as string[], node: root, keys: [] })
  }
  private forget(id: string): void {
    const style = this.styled.get(id); if (style?.kind === 'css') style.fontStyle?.remove()
    const node = this.dom.get(id), kind = this.records.get(id)?.kind
    if (kind === 'tile' || kind === 'mask') this.counts[kind]--
    this.dom.delete(id); this.records.delete(id); this.styled.delete(id); this.styled.delete(`${id}#style`); this.styled.delete(`${id}#adopted`)
    this.frameDocuments.delete(id)
    if (this.fontRequirements.delete(id)) this.fontRequirementsVersion++
    const tile = this.tileUrls.get(id); if (tile) { URL.revokeObjectURL(tile); this.tileUrls.delete(id) }; this.tileBoxes.delete(id)
    if (node) this.ids.delete(node)
  }
  private forgetTree(node: Node): void {
    const id = this.ids.get(node); if (id) this.forget(id)
    for (const child of Array.from(node.childNodes)) this.forgetTree(child)
    if ((node as Element).shadowRoot) this.forgetTree((node as Element).shadowRoot!)
    if ((node as Element).localName === 'iframe') { const nested = (node as HTMLIFrameElement).contentDocument; if (nested) this.forgetTree(nested) }
  }
  private hydrateFrames(frames: Array<{ frame: HTMLIFrameElement; id: string }>, records: Mirror2Record[], scrolls: Array<[Element, number, number]>): void {
    for (let i = 0; i < frames.length; i++) {
      const { frame, id } = frames[i]!
      const nested = frame.contentDocument; if (!nested) throw Error('MP-11: nested mirror unavailable')
      const documentRecord = records.find(r => r.parent === id && r.kind === 'document'); if (!documentRecord) continue
      const subtree = this.subtree(records, documentRecord.id)
      this.records.set(documentRecord.id, documentRecord); this.dom.set(documentRecord.id, nested); this.ids.set(nested, documentRecord.id); this.frameDocuments.add(documentRecord.id)
      const built = this.build(subtree.slice(1), nested, frames, scrolls)
      const html = subtree[1] ? built.get(subtree[1].id) as Element | undefined : undefined
      if (html) { if (nested.documentElement) nested.documentElement.replaceWith(html); else nested.appendChild(html); this.csp(nested) }
      if (documentRecord.adopted?.length) this.setStyled(documentRecord.id, { kind: 'adopted', raw: documentRecord.adopted as string[], node: nested, keys: [] })
      else { this.shadows.delete(nested); nested.adoptedStyleSheets = [] } // a replaced document's sheets do not style the new one
      this.bind(nested)
    }
  }
  private subtree(records: Mirror2Record[], rootId: string): Mirror2Record[] {
    const ids = new Set([rootId]), out: Mirror2Record[] = []
    for (const record of records) if (record.id === rootId || record.parent !== null && ids.has(record.parent)) { ids.add(record.id); out.push(record) }
    return out
  }
  private csp(doc: Document): void {
    // Resource hydration must not turn scroll anchoring into user input.
    doc.documentElement.style.setProperty('overflow-anchor', 'none', 'important')
    const head = doc.head ?? doc.documentElement.insertBefore(doc.createElement('head'), doc.documentElement.firstChild)
    const meta = doc.createElement('meta'); meta.httpEquiv = 'Content-Security-Policy'; meta.content = mirror2SandboxCsp; head.prepend(meta)
  }
  private sheets = new Map<string, string>()
  private sheetCache = new Mirror2SheetCache()
  // MP-08/MP-10/MP-11: hidden preparation never steals focus or emits input.
  setInteractive(enabled: boolean, focus = false): void {
    this.interactive = enabled; this.frame.inert = !enabled
    if (enabled && focus) this.takeFocus(this.kernelFocus)
  }
  holdForHandover(): void { this.navigation.hold(); this.setInteractive(false) }
  handover(focus = false): void { this.navigation.release(); this.setInteractive(true, focus); this.navigationPaint() }
  // MP-08/MP-10/MP-11: fixed reasons only, no node/resource identities.
  private handoverStage = ''
  private documentAt = 0
  private resourceAt = 0
  private nativeFontFailures = new Set<string>()
  private nativeFontFailureKey = '[]'
  private fontRequirements = new Map<string, string[] | null>()
  private fontRequirementsVersion = 0
  private fontRequirementsNegotiated = false
  requireNativeFonts(negotiated: boolean): void { this.fontRequirementsNegotiated = negotiated; this.fontRequirementsVersion++ }
  private handoverDiagnostic(reason: string): void {
    if (this.handoverStage === reason) return
    this.handoverStage = reason; this.timed(`handover_${reason}`, performance.now())
  }
  async readyForHandover(): Promise<boolean> {
    const doc = this.doc, documentId = this.documentId, proofSequence = this.sequence
    if (!doc || !this.navigation.readyToPresent) { this.handoverDiagnostic('wait_content'); return false }
    if (this.slices.size) { this.handoverDiagnostic('wait_slices'); return false }
    // An image the page never loads (failed or never fetched, <img> or CSS) holds the switch only
    // until resources stop arriving for a while.
    const imageHold = performance.now() - Math.max(this.documentAt, this.resourceAt) < IMAGE_HOLD_MS
    if (imageHold) for (const [id, record] of this.records) {
      if (!record.res || this.resources.has(record.res)) continue
      const element = this.dom.get(id) as Element | undefined
      const box = element?.nodeType === 1 ? element.getBoundingClientRect() : null
      if (box && box.width > 0 && box.height > 0 && box.right > 0 && box.bottom > 0 && box.left < this.frame.clientWidth && box.top < this.frame.clientHeight) { this.handoverDiagnostic('wait_image'); return false }
    }
    // Fonts and visible CSS imagery must not pop into place after the switch.
    const nativeFailures = this.nativeFontFailures, fontVersion = this.fontRequirementsVersion, legacyChecks = new Set<Document>()
    for (const [id, record] of this.records) if (record.kind === 'document') {
      const document = this.dom.get(id) as Document | undefined
      if (document?.nodeType !== 9) continue
      if (document.fonts.status === 'loading') this.handoverDiagnostic('wait_fonts')
      const required = this.fontRequirements.get(id) ?? (this.fontRequirementsNegotiated || this.fontRequirements.size ? null : undefined)
      const fonts = await mirror2DocumentFontsReady(document, required, document === doc ? nativeFailures : new Set())
      if (!fonts.ready) { this.handoverDiagnostic('wait_fonts'); return false }
      if (fonts.checkRendered) legacyChecks.add(document)
    }
    if (this.disposed || this.doc !== doc || this.documentId !== documentId || this.sequence !== proofSequence || nativeFailures !== this.nativeFontFailures || fontVersion !== this.fontRequirementsVersion) return false
    for (const node of this.dom.values()) if (node.nodeType === 1) {
      const element = node as Element, box = element.getBoundingClientRect()
      if (box.width <= 0 || box.height <= 0 || box.right <= 0 || box.bottom <= 0 || box.left >= this.frame.clientWidth || box.top >= this.frame.clientHeight) continue
      const style = element.ownerDocument.defaultView?.getComputedStyle(element)
      if (legacyChecks.has(element.ownerDocument) && style && !mirror2RenderedFontReady(element, style)) { this.handoverDiagnostic('wait_fonts'); return false }
      if (imageHold && style?.visibility === 'visible' && [style.backgroundImage, style.maskImage, style.borderImageSource, style.listStyleImage].some(value=>value.includes(this.empty))) { this.handoverDiagnostic('wait_css_image'); return false }
    }
    if ([...doc.images].some(image=>image.hasAttribute('src')&&!image.complete)) this.handoverDiagnostic('wait_decode')
    await Promise.all([...doc.images].filter(image=>image.hasAttribute('src')).map(image=>image.decode().catch(()=>{})))
    const ready = !this.disposed && this.doc === doc && this.documentId === documentId && this.sequence === proofSequence && fontVersion === this.fontRequirementsVersion && !this.slices.size
    this.handoverDiagnostic(ready ? 'ready' : 'wait_document')
    return ready
  }
  knownSheets(): string[] { return this.sheetCache.digests() }
  async apply(wire: Mirror2Packet): Promise<void> {
    const started = performance.now()
    // MP-08/MP-10/MP-11: attribute reset long tasks before slicing the work.
    // Numeric phases contain no document, source text or resource identities.
    let phase = started
    const mark = (stage: string): void => { if (wire.reset) this.timed(stage, phase); phase = performance.now() }
    if (this.disposed || !this.doc) throw Error('MP-08: mirror unavailable')
    if (!wire.reset && (wire.base_sequence !== this.sequence || wire.document_id !== this.documentId)) throw Error('MP-11: mirror lost base')
    const packet = resolveMirror2Sheets(wire, this.sheets, this.sheetCache.texts)
    validateMirror2Packet(packet, this.records)
    mark('apply_validate')
    const fontFailureKey = JSON.stringify(packet.font_failures ?? [])
    if (packet.reset || fontFailureKey !== this.nativeFontFailureKey) { this.nativeFontFailureKey = fontFailureKey; this.nativeFontFailures = new Set(packet.font_failures ?? []) }
    this.sheetCache.remember(wire.sheets)
    // Resource keys are per document; a new document starts an empty map. A
    // reset restarts resources still in slices (the kernel sends them again).
    if (packet.reset && packet.document_id !== this.documentId) { for (const url of [...this.resources.values(), ...this.sprites.values()]) if (url.startsWith('blob:')) URL.revokeObjectURL(url); this.resources.clear(); this.sprites.clear() }
    if (packet.reset && this.documentId && packet.document_id !== this.documentId) {
      this.wheel.cancel(); this.scrollTargets.clear()
      if (this.scrollFrame) { cancelAnimationFrame(this.scrollFrame); this.scrollFrame = 0 }
      this.navigationAt = performance.now()
      await this.navigation.prepare()
      if (this.disposed) throw Error('MP-08: mirror closed during navigation')
      this.doc = this.frame.contentDocument
      if (!this.doc) throw Error('MP-11: mirror sandbox unavailable')
      this.bind(this.doc)
      this.focusSent = null; this.pointerFocus = null; this.focusHold = null
      this.selectionDoc = null; this.selectionSent = ''; this.selectionHold = null; this.selectionFailure = false
    }
    if (packet.reset) this.slices.clear()
    const complete: Mirror2Resource[] = []
    for (const slice of packet.resources) { const resource = this.assemble(slice); if (resource) { await this.addResource(resource); complete.push(resource) } }
    if (this.disposed || !this.doc) throw Error('MP-08: mirror closed during validation')
    // A bounded loading-document resnapshot changes node ids, not the user's
    // root wheel intent. Preserve it and submit only under the new epoch.
    const rootScroll = packet.reset && packet.document_id === this.documentId && this.rootScrolling()
      ? this.wheel.rootTarget(this.doc) ?? [this.frame.contentWindow!.scrollX, this.frame.contentWindow!.scrollY] : null
    if (packet.reset && packet.document_id !== this.documentId) { this.localRootScrollAt = -Infinity; this.releasePointer() }
    const savedRange = packet.reset && packet.document_id === this.documentId && (this.pointerDown || this.selectionHold) ? rememberMirror2Range(this.doc, node => this.ids.has(node)) : null
    let rangeRestored = false
    mark('apply_prepare')
    this.applying = true
    const scrolls: Array<[Element, number, number]> = []
    try {
      if (packet.reset) { this.resetSerial++; this.reset(packet, scrolls); this.focusSent = null; this.pointerFocus = null; this.focusHold = null; this.focusWithheld = undefined; this.selectionSent = ''; this.selectionHold = null; this.selectionFailure = false }
      mark('apply_build')
      for (const op of packet.ops ?? []) this.op(op, scrolls)
      mark('apply_ops')
      for (const resource of complete) for (const entry of this.styled.values()) if (entry.keys.includes(resource.key)) this.restyle(entry)
      for (const resource of complete) for (const [id, record] of this.records) if (record.res === resource.key) { const node = this.dom.get(id); if (node?.nodeType === 1) this.image(id, node as Element, record.res) }
      for (const tile of packet.tiles) this.tile(tile)
      mark('apply_resources')
      const states = this.states; this.states = []
      for (const element of states) this.uaState(element)
      this.targets(packet.fragment ?? null)
      this.doc.documentElement.style.setProperty('overflow-anchor', 'none', 'important')
      for (const id of this.frameDocuments) { const d = this.dom.get(id) as Document | undefined; if (d?.nodeType === 9) d.documentElement.style.setProperty('overflow-anchor', 'none', 'important') }
      // The viewer owns scroll while it scrolls; the kernel's positions (the latest per
      // scroller, kept while deferred) apply on reset or once the viewer settles.
      mark('apply_state')
      if (packet.reset) { this.deferred.clear(); this.wheel.cancel() }
      for (const [element, x, y] of scrolls) this.deferred.set(element, [x, y])
      // MP-08/MP-10/MP-11: hidden native preparation keeps the latest
      // scroller positions, without laying out the growing tree per packet.
      if ((!this.navigation.holding || packet.snapshot_complete === true) && (packet.reset || !this.scrolling())) {
        const pending = this.deferred; this.deferred = new Map()
        for (const [element, [x, y]] of pending as Map<Element, [number, number]>) {
          // A frame document scrolls its viewport (its root element: a quirks-mode viewer document would not).
          const doc = (element as Node).nodeType === 9 ? element as unknown as Document : element === element.ownerDocument.documentElement ? element.ownerDocument : null
          if (doc) { const view = doc.defaultView; view?.scrollTo(x, y); if (view) this.applied.set(doc, [view.scrollX, view.scrollY]); continue }
          if (element.scrollLeft !== x) element.scrollLeft = x; if (element.scrollTop !== y) element.scrollTop = y
          this.applied.set(element, [element.scrollLeft, element.scrollTop])
        }
        const view = this.frame.contentWindow!, position = rootScroll ?? packet.scroll; view.scrollTo(position[0]!, position[1]!); this.applied.set(this.doc, [view.scrollX, view.scrollY])
      }
      mark('apply_scroll')
      for (const entry of this.styled.values()) if (entry.kind === 'css') entry.fontStyle?.attach()
      mark('apply_fonts')
      this.navigation.present(packet.snapshot_complete === true)
      mark('apply_present')
      this.navigationPaint()
      this.frame.inert = !this.interactive
      if (packet.document_id !== this.documentId) this.documentAt = performance.now()
      this.sequence = packet.sequence; this.documentId = packet.document_id
      // A focus taken by a pointer still pressed is pending input too (it is sent with its click or drag end).
      this.kernelFocus = packet.focused
      rangeRestored = restoreMirror2Range(this.doc, savedRange, node => this.ids.has(node), node => this.records.get(this.ids.get(node) ?? '')?.kind === 'mask')
      const selectionKey = packet.selection ? JSON.stringify({kind:'selection',anchor_id:packet.selection.anchor_id,anchor_offset:packet.selection.anchor_offset,focus_id:packet.selection.focus_id,focus_offset:packet.selection.focus_offset}) : ''
      if (this.selectionHold && (selectionKey === this.selectionHold.key || packet.sequence > this.selectionHold.until)) this.selectionHold = null
      const hold = this.focusHold
      if (hold && (packet.focused !== hold.previous || packet.sequence > hold.until)) this.focusHold = null
      this.focusWithheld = this.pendingInputs && !this.pointerFocus && !this.focusHold ? packet.focused : undefined
      if (!this.pendingInputs && !this.pointerFocus && !this.focusHold && !this.pointerDown) {
        this.takeFocus(packet.focused)
        if (!this.selectionHold && !rangeRestored) {
          // Once the local hold expires, the kernel's selection is authoritative,
          // including a cleared range. A focused editing control keeps its own
          // range because removeAllRanges would collapse its active selection.
          const s = packet.selection, a = s && this.dom.get(s.anchor_id), b = s && this.dom.get(s.focus_id), doc = a && b ? a.ownerDocument : null
          if (this.selectionDoc && this.selectionDoc !== doc && !this.editing(this.selectionDoc)) this.selectionDoc.getSelection()?.removeAllRanges()
          this.selectionSent = selectionKey
          if (doc) doc.getSelection()?.setBaseAndExtent(a!, s!.anchor_offset, b!, s!.focus_offset)
          this.selectionDoc = doc
        }
      }
      mark('apply_layout')
    } finally { this.applying = false }
    if (rangeRestored) this.syncSelection(this.doc)
    if (rootScroll && (rootScroll[0] !== packet.scroll[0] || rootScroll[1] !== packet.scroll[1])) this.enqueue({kind:'scroll_to',node_id:null,x:rootScroll[0]!,y:rootScroll[1]!})
    Object.assign(this.frame.dataset, { mirrorSequence: String(packet.sequence), mirrorRegions: String(this.counts.tile), mirrorMasks: String(this.counts.mask) })
    this.timed(packet.reset ? 'apply_reset' : 'apply_delta', started)
  }
  private reset(packet: Mirror2Packet, scrolls: Array<[Element, number, number]>): void {
    const doc = this.doc!
    for (const url of this.tileUrls.values()) URL.revokeObjectURL(url)
    this.tileUrls.clear(); this.tileBoxes.clear(); this.dom.clear(); this.records.clear(); this.styled.clear(); this.frameDocuments.clear(); this.ids = new WeakMap(); this.counts = { tile: 0, mask: 0 }
    this.fontRequirements.clear(); this.fontRequirementsVersion++
    const records = packet.nodes ?? [], frames: Array<{ frame: HTMLIFrameElement; id: string }> = []
    const root = records[0]!
    const main = this.subtree(records, root.id)
    // The top document record maps onto the sandbox document itself.
    this.records.set(root.id, root); this.dom.set(root.id, doc); this.ids.set(doc, root.id)
    const built = this.build(main.slice(1), doc, frames, scrolls)
    const html = main[1] ? built.get(main[1].id) as Element | undefined : undefined
    if (html && packet.document_id !== this.documentId) { doc.documentElement.replaceWith(html); this.csp(doc) }
    else if (html) {
      const root = doc.documentElement
      // Keep the sandbox root stable; all content and attributes still come
      // exclusively from this validated snapshot, including protection resets.
      for (const attribute of [...root.attributes]) root.removeAttribute(attribute.name)
      for (const attribute of [...html.attributes]) root.setAttribute(attribute.name, attribute.value)
      root.replaceChildren(...html.childNodes)
      const id = main[1]!.id
      this.dom.set(id, root); this.ids.set(root, id)
      const style = this.styled.get(`${id}#style`); if (style?.kind === 'attr') style.node = root
      this.csp(doc)
    }
    if (root.adopted?.length) this.setStyled(root.id, { kind: 'adopted', raw: root.adopted as string[], node: doc, keys: [] })
    else { this.shadows.delete(doc); doc.adoptedStyleSheets = [] }
    this.hydrateFrames(frames, records, scrolls)
  }
  private op(op: Mirror2Op, scrolls: Array<[Element, number, number]>): void {
    const node = this.dom.get(op.id)
    if (!node) return // removed earlier in this packet
    switch (op.op) {
      case 'fonts': this.fontRequirements.set(op.id, op.required); this.fontRequirementsVersion++; this.shadowFonts(node as Document, op.absent ?? []); break
      case 'children': {
        const frames: Array<{ frame: HTMLIFrameElement; id: string }> = []
        // A frame's child is its document: rebuild the frame's own document.
        if (this.records.get(op.id)?.kind === 'frame') {
          const nested = (node as HTMLIFrameElement).contentDocument
          if (nested) this.forgetTree(nested)
          this.hydrateFrames([{ frame: node as HTMLIFrameElement, id: op.id }], op.nodes, scrolls)
          break
        }
        const doc = node.nodeType === 9 ? node as Document : node.ownerDocument!
        const fresh = this.build(op.nodes, doc, frames, scrolls)
        const container: Node = node
        const desired: Node[] = [], shadowIds: string[] = []
        for (const id of op.children) {
          const record = this.records.get(id)
          if (record?.kind === 'shadow') { shadowIds.push(id); continue }
          const child = fresh.get(id) ?? this.dom.get(id); if (child) desired.push(child)
        }
        // A kept descendant may still be inside a wrapper we are about to retire.
        // Move it first, including from a shadow root or frame, so its index survives.
        for (const child of desired) {
          let ancestor: Node | null = child.parentNode
          while (ancestor && ancestor !== container) ancestor = ancestor.parentNode
            ?? (ancestor.nodeType === 11 ? (ancestor as ShadowRoot).host ?? null
              : ancestor.nodeType === 9 ? (ancestor as Document).defaultView?.frameElement ?? null : null)
          if (ancestor === container && child.parentNode !== container) container.appendChild(child)
          const record = this.records.get(this.ids.get(child) ?? ''); if (record) record.parent = op.id
        }
        const keep = new Set(desired)
        // Forgotten before removal: a detached iframe no longer exposes its document.
        for (const child of Array.from(container.childNodes)) if (!keep.has(child) && (this.ids.has(child) || child.nodeType !== 1 || !(child as Element).matches?.('meta[http-equiv]'))) { this.forgetTree(child); container.removeChild(child) }
        for (let i = 0; i < desired.length; i++) if (container.childNodes[i] !== desired[i]) container.insertBefore(desired[i]!, container.childNodes[i] ?? null)
        for (const id of shadowIds) { const record = this.records.get(id)!; const fragment = fresh.get(id); if (fragment && node.nodeType === 1) this.attachShadow(node as Element, record, fragment as DocumentFragment) }
        if (node.nodeType === 9 && !(node as Document).head?.querySelector('meta[http-equiv]')) this.csp(node as Document)
        // Changed options (also beneath an optgroup) reset a select's selection: restore its recorded state.
        const select = node.nodeType === 1 ? (node as Element).closest('select') : null, selected = select && this.records.get(this.ids.get(select) ?? '')?.form
        if (selected) this.form(select!, selected)
        this.hydrateFrames(frames, op.nodes, scrolls)
        break
      }
      case 'attr': {
        const element = node as Element; const record = this.records.get(op.id)!
        record.attrs ??= {}
        if (op.value === null) { delete record.attrs[op.name]; if (op.name === 'style') this.styled.delete(`${op.id}#style`); element.removeAttribute(op.name) }
        else { record.attrs[op.name] = op.value; if (op.name === 'style') this.setStyled(op.id, { kind: 'attr', raw: op.value, node: element, keys: [] }); else element.setAttribute(op.name, op.name === 'popover' ? 'manual' : op.value) }
        if (op.name === 'style' && op.value === null) this.opaqueLayout(record, element)
        if (op.name === 'href' && record.res !== undefined && element.localName === 'use') this.image(op.id, element, record.res) // its sprite file + the new fragment
        break
      }
      case 'text': node.textContent = op.text; this.records.get(op.id)!.text = op.text; break
      case 'css': { this.records.get(op.id)!.css = op.css; this.setStyled(op.id, { kind: 'css', raw: op.css, node: node as Element, keys: [] }); break }
      case 'adopted': this.setStyled(op.id, { kind: 'adopted', raw: op.sheets as string[], node: node as Document | ShadowRoot, keys: [] }); break
      case 'form': { const record = this.records.get(op.id)!, form = record.form = { value: '', checked: false, selected_index: -1, selection_start: null, selection_end: null, ...record.form, ...op.form }; this.form(node as Element, form); break }
      case 'scroll': scrolls.push([node as Element, op.scroll[0], op.scroll[1]]); break
      case 'size': { const style = (node as HTMLElement).style; style.setProperty('width', `${op.size[0]}px`, 'important'); style.setProperty('height', `${op.size[1]}px`, 'important'); this.records.get(op.id)!.size = op.size; break }
      case 'res': this.image(op.id, node as Element, op.res); break
      case 'fragment': { const record = this.records.get(op.id)!; if (op.fragment === null) delete record.fragment; else record.fragment = op.fragment; break }
      case 'state': { const record = this.records.get(op.id)!; record.state = { ...record.state, ...op.state }; this.states.push(node as Element); break }
    }
  }
  // Opaque region pixels paint as the element's own background: they scroll and
  // reflow with the mirrored layout instead of floating at a stale viewport box.
  private tile(tile: Mirror2Tile): void {
    const element = this.dom.get(tile.node_id) as HTMLElement | undefined; if (!element) return
    const old = this.tileUrls.get(tile.node_id); if (old) URL.revokeObjectURL(old)
    const url = URL.createObjectURL(new Blob([bytesOf(tile.data_base64) as Uint8Array<ArrayBuffer>], { type: 'image/png' })); this.tileUrls.set(tile.node_id, url)
    this.tileBoxes.set(tile.node_id, { x: tile.x, y: tile.y, width: tile.width, height: tile.height })
    this.paintTile(tile.node_id, element)
  }
  private paintTile(id: string, element: HTMLElement): void {
    const tile = this.tileBoxes.get(id), url = this.tileUrls.get(id); if (!tile || !url) return
    const style = element.style
    style.setProperty('background-image', `url("${url}")`, 'important'); style.setProperty('background-repeat', 'no-repeat', 'important')
    style.setProperty('background-position', `${tile.x}px ${tile.y}px`, 'important'); style.setProperty('background-size', `${tile.width}px ${tile.height}px`, 'important')
  }
  close(options?:{preserveFrame?:boolean}): void {
    this.disposed = true; this.onEmptyPaint = undefined; this.onTiming = undefined; this.sheetCache.clear(); this.sheets.clear(); this.releasePointer(); this.queue=[]; this.pendingInputs=0; this.wheel.close()
    const owner = this.container.ownerDocument
    owner.removeEventListener('pointerup', this.releasePointer, true)
    owner.removeEventListener('pointercancel', this.releasePointer, true)
    owner.defaultView?.removeEventListener('blur', this.releasePointer)
    if(options?.preserveFrame) {this.navigation.preserve();return}
    this.doc = null
    for (const url of [...this.resources.values(), ...this.sprites.values()]) if (url.startsWith('blob:')) URL.revokeObjectURL(url)
    for (const url of this.tileUrls.values()) URL.revokeObjectURL(url)
    URL.revokeObjectURL(this.empty); this.wheel.close(); this.sprites.clear(); this.deferred.clear(); this.resources.clear(); this.slices.clear(); this.tileUrls.clear(); this.tileBoxes.clear(); this.dom.clear(); this.records.clear(); this.styled.clear(); this.navigation.close()
  }
}

export interface Mirror2Transport { prioritize?():()=>void; receiving?(): boolean; protocolVersion: number; request(request: unknown, options?: {onWritten?:()=>void}): Promise<unknown> }
// Protocol 489: the kernel deflates packet bodies in the subscription's
// context (sync flush per packet, a fresh context per reset), so the viewer
// inflates them strictly in sequence order. Inflation is bounded before parsing.
export class Mirror2Inflater {
  private writer: WritableStreamDefaultWriter<BufferSource>
  private reader: ReadableStreamDefaultReader<Uint8Array>
  constructor() { const stream = new DecompressionStream('deflate-raw'); this.writer = stream.writable.getWriter(); this.reader = stream.readable.getReader() }
  async inflate(base64: unknown, size: unknown): Promise<string> {
    if (typeof base64 !== 'string' || !Number.isSafeInteger(size) || (size as number) < 2 || (size as number) > 256 * 1024 * 1024) throw Error('MP-11: mirror packet bounds')
    void this.writer.write(bytesOf(base64) as Uint8Array<ArrayBuffer>).catch(() => {})
    const body = new Uint8Array(size as number); let at = 0
    while (at < body.length) { const { done, value } = await this.reader.read(); if (done || value.length > body.length - at) throw Error('MP-11: mirror packet bounds'); body.set(value, at); at += value.length }
    return new TextDecoder('utf-8', { fatal: true }).decode(body)
  }
  close(): void { void this.writer.abort().catch(() => {}); void this.reader.cancel().catch(() => {}) }
}
type Binding = { tab_id: string; generation: number; device_scale_factor: 1 | 2; layout_width?: number; layout_height?: number }
const GAP_MS = 500
// The oldest outstanding credit is answered within its wait; one overdue by this much was orphaned by the transport.
const LOST_MS = 1000
// Credit-gated push: `credits` long-poll requests stay outstanding; packets
// apply strictly in sequence. A lasting gap or repeated failure asks for a fresh snapshot.
type Renderer = Pick<BrowserMirror2Renderer, 'ready' | 'apply' | 'close' | 'frame'> & Partial<Pick<BrowserMirror2Renderer,'settleInput' | 'painted' | 'acceptsInput' | 'onEmptyPaint' | 'onTiming' | 'knownSheets' | 'setInteractive' | 'readyForHandover' | 'holdForHandover' | 'handover' | 'requireNativeFonts'>>
// A failure streak that outlives this with no packet is terminal (the runtime then
// shows protected video and retries later); shorter streaks back off.
const FAILING_MS = 15_000
export async function attachBrowserMirror2(transport: Mirror2Transport, container: HTMLElement, binding: Binding, handlers: { failure(error: unknown): void; preparing?(packet: Mirror2Packet): void; prepareReset?(packet: Mirror2Packet): boolean; inputFailure?(error: unknown): void; packet?(packet: Mirror2Packet): void; presented?(packet: Mirror2Packet): void; timing?(sample:Mirror2Timing):void; diagnostic?(stage:string,error:unknown,sequence?:number):void }, { credits = CREDIT_WINDOW, waitMs = 2000, failingMs = FAILING_MS, preserveFrameOnFailure=false, renderer: createRenderer = (container: HTMLElement, send: ConstructorParameters<typeof BrowserMirror2Renderer>[1]): Renderer => new BrowserMirror2Renderer(container, send) } = {}) {
  if (!Number.isInteger(transport.protocolVersion) || transport.protocolVersion < browserMirror2MinimumProtocolVersion) throw Error('MP-08: DOM mirror v2 requires protocol 550')
  const request = async (command: unknown, onWritten?:()=>void): Promise<any> => { const response = await transport.request({ KernelBrowser: { command } },onWritten?{onWritten}:undefined) as { KernelBrowser?: { result?: unknown } }; if (!response.KernelBrowser?.result) throw Error('MP-08: invalid mirror response'); return response.KernelBrowser.result }
  const releaseHold = transport.prioritize?.() ?? (() => {})
  let priorityReleased = false
  const releasePriority = () => { if (!priorityReleased) { priorityReleased = true; releaseHold() } }
  const subscribe = { op: 'mirror_subscribe', ...binding, wire: 2, font_requirements:true, ...(transport.protocolVersion >= 567 ? {font_absent:true} : {}), ...(handlers.preparing ? {navigation_preview:true} : {}) }
  const subscribed = await request(subscribe).catch(error => { releasePriority(); throw error }); const subscription_id = subscribed.subscription_id as string
  // Four credits while the page or the viewer is active, one when idle (one heartbeat per wait).
  let closed = false, inflight = 0, applied = 0, wantReset = true, resetOutstanding = false, wantResend = false, resendOutstanding = false, activeAt = -Infinity, gapSince = 0, failures = 0, failingSince = 0, failGeneration = 0
  const buffered = new Map<number, Mirror2WirePacket>()
  const outstanding: Array<{ reset: boolean, oldestAt: number }> = [] // credits in flight, oldest first
  // In sequence order: inflate in the context, then restore what a compact delta implies.
  let inflater: Mirror2Inflater | null = null, base: Mirror2Packet | null = null
  const expand = async (wire: Mirror2WirePacket): Promise<Mirror2Packet> => {
    if (wire.reset) { inflater?.close(); inflater = null }
    let body: Mirror2WirePacket = wire
    if (wire.encoding !== undefined) {
      if (wire.encoding !== 'deflate') throw Error('MP-11: mirror packet bounds')
      const text = await (inflater ??= new Mirror2Inflater()).inflate(wire.packet_base64, wire.packet_bytes)
      body = JSON.parse(text) as Mirror2WirePacket
      if (body.sequence !== wire.sequence || Boolean(body.reset) !== Boolean(wire.reset)) throw Error('MP-11: foreign mirror packet')
    }
    const media = { resources: wire.resources ?? [], tiles: wire.tiles ?? [] }
    if (wire.reset) {
      const packet = decodeMirror2Packet({ ...body, ...media } as Mirror2Packet)
      if (packet.subscription_id !== subscription_id || packet.tab_id !== binding.tab_id || packet.generation !== binding.generation) throw Error('MP-11: foreign mirror packet')
      return packet
    }
    if (!base) throw Error('MP-08: mirror lost base')
    const header = (name: 'scroll' | 'focused' | 'selection' | 'fragment' | 'snapshot_complete' | 'font_failures') => Object.hasOwn(body, name) ? body[name] : base![name] ?? (name === 'snapshot_complete' ? false : name === 'font_failures' ? [] : null)
    return decodeMirror2Packet({ ...body, ...media, subscription_id, tab_id: binding.tab_id, generation: binding.generation, document_id: base.document_id, base_sequence: wire.sequence - 1, reset: false,
      css_width: base.css_width, css_height: base.css_height, device_scale_factor: base.device_scale_factor, scroll: header('scroll'), focused: header('focused'), selection: header('selection'), fragment: header('fragment'), snapshot_complete: header('snapshot_complete'), font_failures: header('font_failures') } as Mirror2Packet)
  }
  // MP-08/MP-11: undo/redo keep the mirror's frame, focus and epoch admission.
  const renderer = createRenderer(container, async (action, epoch, onWritten) => { activeAt = performance.now(); fill(); try { return await request({ op: 'mirror_input', tab_id: binding.tab_id, generation: binding.generation, document_id: epoch.document_id, subscription_id, sequence: epoch.sequence, action: 'modifiers' in action ? { kind: 'key', key: action.modifiers & 8 ? 'Ctrl+Shift+Z' : 'Ctrl+Z' } : action },onWritten) } catch(error) { if(!closed){handlers.diagnostic?.('mirror_input_refused',error,epoch.sequence);handlers.inputFailure?.(error)} throw error } })
  renderer.requireNativeFonts?.(subscribed.font_requirements === true)
  renderer.onTiming = sample => { if (sample.stage === 'mirror_navigation_paint') releasePriority(); handlers.timing?.(sample) }
  try { await renderer.ready() } catch (error) { releasePriority(); renderer.close(); await request({ op: 'mirror_close', subscription_id, generation: binding.generation }).catch(() => {}); throw error }
  // Notify consumers when the empty-page timer presents the last applied
  // frame. A silent/stalled credit must not keep the previous View attached.
  renderer.onEmptyPaint = () => {
    if (!closed && renderer.painted !== false) releasePriority()
    if (!closed && base && !base.fallback && base.snapshot_complete === true && renderer.painted !== false && renderer.frame.dataset.mirrorSequence === String(base.sequence)) handlers.presented?.(base)
  }
  let chain: Promise<void> = Promise.resolve()
  // Packets apply one at a time. A failed application rejects only its own
  // caller (which asks for a reset); later packets still drain.
  const drain = (): Promise<void> => { const run = chain.then(async () => {
    for (let wire = [...buffered.values()].find(p => p.reset || p.sequence === applied + 1); wire; wire = [...buffered.values()].find(p => p.reset || p.sequence === applied + 1)) {
      for (const seq of buffered.keys()) if (seq <= wire.sequence) buffered.delete(seq)
      const next = await expand(wire)
      if (next.fallback === 'navigation_preview' && handlers.preparing) { renderer.holdForHandover?.(); handlers.preparing(next) }
      else if (!next.fallback) {
        // MP-08/MP-10/MP-11: native fallback recovery can start with an
        // ordinary reset. Hold it before any prefix reaches the visible view.
        if (next.reset && handlers.prepareReset?.(next) === true) renderer.holdForHandover?.()
        await renderer.apply(next)
      }
      // Only an applied packet ends a failure streak (a reply that cannot be applied does not).
      base = next; failures = 0; failingSince = 0
      if (next.fallback && next.fallback !== 'navigation_preview' || renderer.painted === undefined) releasePriority()
      if (next.reset || next.ops?.length || next.resources.length || next.tiles.length) activeAt = performance.now()
      applied = next.sequence
      if (next.reset) { wantReset = false; wantResend = false }
      if (next.fallback !== 'navigation_preview' || !handlers.preparing) handlers.packet?.(next)
    }
    if (buffered.size > 8) { buffered.clear(); wantReset = true }
    // A gap is a reply lost with a dropped socket (or one overtaken by milliseconds: its duplicate is ignored):
    // the missing packet is asked for at once (protocol 501 resend). A gap that outlives its resend resets.
    if (buffered.size && transport.receiving?.()) setTimeout(() => { if (!closed) void drain().catch(() => { wantReset = true; fill() }) }, GAP_MS)
    if (!buffered.size) gapSince = 0
    else if (!gapSince && !transport.receiving?.()) {
      const since = gapSince = performance.now(); wantResend = true; fill()
      const check = (): void => { if (gapSince !== since || !buffered.size || closed) return; if (resendOutstanding || transport.receiving?.()) { setTimeout(check, GAP_MS); return } buffered.clear(); gapSince = 0; wantReset = true; fill() }
      setTimeout(check, GAP_MS)
    }
  }); chain = run.catch(() => { buffered.clear(); gapSince = 0 }); return run }
  // A retired subscription or generation never recovers by itself.
  // Stopping local credits and releasing the kernel subscription are separate: the latter happens once.
  let remoteClosed: Promise<void> | null = null
  const closeRemote = (): Promise<void> => remoteClosed ??= request({ op: 'mirror_close', subscription_id, generation: binding.generation }).then(() => {}, () => {})
  const terminal = (error: unknown): boolean => error instanceof Error && /stale or foreign mirror|MD-2: (?:browser operation failed: |host browser operation failed; refresh state or check host browser readiness \()[A-Za-z0-9_ .:-]*\bmirror_subscription_retired\b/.test(error.message)
  const retired = (error: unknown): boolean => error instanceof Error && /MD-2: (?:browser operation failed: |host browser operation failed; refresh state or check host browser readiness \()Error stale_document_reference\b/.test(error.message)
  const fatal = (error: unknown): boolean => error instanceof Error && /MP-11: (invalid|unsafe|foreign|executable|active|protected|mirror resource digest|mirror packet bounds)/.test(error.message)
  const credit = (): void => {
    const reset = wantReset && !resetOutstanding, resend = !reset && wantResend && !resendOutstanding && applied > 0, generation = failGeneration
    if (reset) resetOutstanding = true
    if (resend) { resendOutstanding = true; wantResend = false }
    inflight++
    let answered = false
    // A credit whose request the transport neither answers nor fails (a socket replaced without failing it) would
    // hold its packet until the request timeout: once it is the oldest and overdue, a resend fetches that packet.
    // The kernel starts a queued credit's wait when its predecessor is answered: the oldest is overdue a wait after that.
    const entry = { reset, oldestAt: outstanding.length ? 0 : performance.now() }; outstanding.push(entry)
    const overdue = (): void => { if (closed || !outstanding.includes(entry)) return; if (outstanding[0] !== entry || entry.reset || transport.receiving?.()) { setTimeout(overdue, LOST_MS); return } const left = entry.oldestAt + waitMs + LOST_MS - performance.now(); if (left > 0) { setTimeout(overdue, left); return } if (!resendOutstanding && applied) { wantResend = true; fill() } }
    setTimeout(overdue, waitMs + LOST_MS)
    request({ op: 'mirror_next', subscription_id, generation: binding.generation, after_sequence: reset ? 0 : applied, drift_nodes: [], ...(renderer.knownSheets?.().length ? { known_sheets: renderer.knownSheets() } : {}), wait_ms: waitMs, ...(resend ? { resend: true } : {}) })
      .then(async (packet: Mirror2WirePacket) => {
        answered = true
        if (!packet || packet.wire !== 2 || !Number.isSafeInteger(packet.sequence) || packet.sequence <= 0) throw Error('MP-11: foreign mirror packet')
        if (packet.reset) resetOutstanding = false
        // A replayed or late copy of an applied packet is not a gap.
        if (packet.sequence > applied) buffered.set(packet.sequence, packet)
        await drain()
      })
      .catch(error => {
        // A credit that failed (a dropped socket, a relay re-authentication) asks for a resend: the kernel may still
        // hold its long poll, which the resend's hurry ends, and the packet that poll issued comes back from history
        // (no full snapshot). A reply that could not be applied asks for a reset.
        // Credits sent before a failure that fail too (the same socket) count as that one failure for the backoff.
        if (reset) resetOutstanding = false
        // A credit refused because a navigation retired its document was answered, not lost: the next credit
        // takes the new document's reset. A resend would fetch that reset again behind the one in flight.
        if (retired(error)) return
        if (generation === failGeneration) { failures++; failGeneration++; if(!closed) handlers.diagnostic?.(answered ? "mirror_apply_retry" : "mirror_credit_retry",error,applied) }
        if (answered) wantReset = true; else wantResend = true
        failingSince ||= performance.now()
        if (!closed && (fatal(error) || terminal(error) || performance.now() - failingSince > failingMs)) { closed = true; releasePriority(); handlers.diagnostic?.(fatal(error) ? "mirror_fatal" : terminal(error) ? "mirror_binding_retired" : "mirror_failing_streak",error,applied); inflater?.close(); renderer.close({preserveFrame:preserveFrameOnFailure}); handlers.failure(error); void closeRemote() }
      })
      .finally(() => { outstanding.splice(outstanding.indexOf(entry), 1); if (outstanding[0] && !outstanding[0].oldestAt) outstanding[0].oldestAt = performance.now(); if (resend) resendOutstanding = false; inflight--; if (!closed) setTimeout(fill, failures ? Math.min(4000, 250 * 2 ** (failures - 1)) : 0) })
  }
  // Pipelined credits start once a snapshot is applied; a reset or a resend is a single credit and never waits
  // behind credits in flight (the kernel ends their wait).
  const fill = (): void => {
    if (!closed && applied && wantReset && !resetOutstanding) credit()
    else if (!closed && applied && wantResend && !resendOutstanding) credit()
    // A packet received before its base (a small one overtaking a large one) still holds its credit: the kernel
    // never runs further ahead of the applied base than the window.
    while (!closed && inflight + buffered.size < (wantReset || !applied || performance.now() - activeAt > 3000 ? 1 : credits)) credit()
  }
  fill()
  const firstWait=setTimeout(()=>{if(!closed&&!applied)handlers.diagnostic?.("mirror_first_packet_wait","no_applied_packet",0)},8000)
  return {
    renderer,
    // A fresh drain is queued after all earlier credits. Call only after native
    // input receipts; unlike resend it never reuses an old observer packet.
    async synchronize(): Promise<Mirror2Packet> {
      if (closed) throw Error('MP-08: mirror closed')
      const wire = await request({op:'mirror_next',subscription_id,generation:binding.generation,after_sequence:applied,drift_nodes:[],wait_ms:0,flush:true}) as Mirror2WirePacket
      if (!wire || wire.wire !== 2 || !Number.isSafeInteger(wire.sequence) || wire.sequence <= 0) throw Error('MP-11: foreign mirror packet')
      if (wire.sequence > applied) buffered.set(wire.sequence,wire)
      await drain()
      const deadline = performance.now()+8000
      while (!closed && applied < wire.sequence && performance.now()<deadline) await new Promise(resolve=>setTimeout(resolve,10))
      if (closed || applied < wire.sequence || !base) throw Error('MP-08: mirror handover not synchronized')
      return base
    },
    async settleInput() {
      if(closed)throw Error('MP-08: mirror closed')
      await renderer.settleInput?.()
      if(closed)throw Error('MP-08: mirror closed')
    },
    async close(options?:{preserveFrame?:boolean}) { releasePriority(); clearTimeout(firstWait); if (!closed) { closed = true; inflater?.close() } renderer.close(options); await closeRemote() },
    takeover: () => request({ op: 'display_takeover', tab_id: binding.tab_id, generation: binding.generation }),
    release: () => request({ op: 'display_release', tab_id: binding.tab_id, generation: binding.generation }),
  }
}
