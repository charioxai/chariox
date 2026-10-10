// MP-08/MP-10/MP-11: DOM mirror v2 renderer (local protocol 489). The page's own
// sanitized stylesheets and attributes are rebuilt in a script-free sandbox;
// deltas apply in place. No origin I/O: resources arrive as kernel bytes.
import { validateMirror2Packet, decodeMirror2Packet, resolveMirror2Sheets, mirror2SandboxCsp } from './browser-mirror2-security.js'
import type { Mirror2Action, Mirror2Form, Mirror2Op, Mirror2Packet, Mirror2Record, Mirror2Resource, Mirror2Tile, Mirror2WirePacket } from './browser-mirror2-types.js'
export * from './browser-mirror2-types.js'
export { mirror2SandboxCsp, validateMirror2Packet, decodeMirror2Packet } from './browser-mirror2-security.js'
export const browserMirror2MinimumProtocolVersion = 489
const NS = { svg: 'http://www.w3.org/2000/svg', math: 'http://www.w3.org/1998/Math/MathML' } as const
const resourcePattern = /url\("mr:(r[0-9]{1,9})"\)/g
type Styled = { kind: 'css' | 'attr'; raw: string; node: Element; keys: string[] } | { kind: 'adopted'; raw: string[]; node: Document | ShadowRoot; keys: string[] }

function bytesOf(data: string): Uint8Array { return Uint8Array.from(atob(data), c => c.charCodeAt(0)) }
async function digest(bytes: Uint8Array): Promise<string> {
  return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes as Uint8Array<ArrayBuffer>)), b => b.toString(16).padStart(2, '0')).join('')
}

export class BrowserMirror2Renderer {
  readonly frame: HTMLIFrameElement
  readonly timings: Array<{ stage: string; duration_ms: number; ended_ms: number }> = []
  private doc: Document | null = null
  private loaded: Promise<void>
  private disposed = false
  private applying = false
  private dom = new Map<string, Node>()
  private records = new Map<string, Mirror2Record>()
  private ids = new WeakMap<Node, string>()
  private resources = new Map<string, string>()
  private slices = new Map<string, Mirror2Resource>() // resources still arriving in slices
  private styled = new Map<string, Styled>()
  private tileUrls = new Map<string, string>()
  private selects: Array<[Element, Mirror2Form]> = []
  private bound = new WeakSet<Document>() // a retired frame document is not kept alive by its binding
  private empty: string
  sequence = 0
  documentId = ''
  private counts = { tile: 0, mask: 0 } // public diagnostics on the frame element (data-mirror-*)
  private pendingInputs = 0
  private queue: Array<{ action: Mirror2Action; epoch: { sequence: number; document_id: string } }> = []
  private sending = false
  // Viewer-owned scroll (plan 4.1): kernel echoes wait until the viewer settles.
  private localScrollAt = -Infinity
  private applied = new WeakMap<Node, [number, number]>() // kernel-applied positions (their scroll events are not viewer input)
  private deferred = new Map<Node, [number, number]>() // latest kernel position per scroller that arrived while the viewer scrolled
  constructor(private container: HTMLElement, private send: (action: Mirror2Action, epoch: { sequence: number; document_id: string }) => Promise<unknown>, _failure?: (error: unknown) => void) {
    const owner = container.ownerDocument
    this.empty = URL.createObjectURL(new Blob([]))
    this.frame = owner.createElement('iframe')
    this.frame.setAttribute('sandbox', 'allow-same-origin') // scripts NEVER enabled
    this.frame.setAttribute('referrerpolicy', 'no-referrer')
    this.frame.setAttribute('title', 'Mirrored kernel browser page')
    this.frame.style.cssText = 'width:1280px;height:800px;border:0;display:block'
    this.frame.srcdoc = `<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="${mirror2SandboxCsp}"></head><body></body></html>`
    this.loaded = new Promise<void>(resolve => this.frame.addEventListener('load', () => resolve(), { once: true }))
    container.append(this.frame)
  }
  async ready(): Promise<void> {
    await this.loaded
    if (this.disposed) throw Error('MP-08: mirror closed')
    this.doc = this.frame.contentDocument
    if (!this.doc) throw Error('MP-11: mirror sandbox unavailable')
    this.bind(this.doc)
  }
  private timed(stage: string, at: number): void { this.timings.push({ stage, duration_ms: performance.now() - at, ended_ms: performance.timeOrigin + performance.now() }); if (this.timings.length > 2048) this.timings.splice(0, 1024) }
  // ---- input: node-addressed, the kernel re-validates everything live.
  private enqueue(action: Mirror2Action): void {
    if (this.applying || this.disposed || !this.documentId) return
    const epoch = { sequence: this.sequence, document_id: this.documentId }
    // Ordered; a queued scroll position for the same scroller is replaced, not appended.
    const last = this.queue.at(-1)
    if (action.kind === 'scroll_to' && last?.action.kind === 'scroll_to' && last.action.node_id === action.node_id) { last.action = action; last.epoch = epoch; return }
    this.queue.push({ action, epoch }); this.pendingInputs++
    void this.pump()
  }
  private async pump(): Promise<void> {
    if (this.sending) return
    this.sending = true
    try {
      for (let next = this.queue.shift(); next; next = this.queue.shift()) {
        // A refused input (stale epoch, changed or protected target) is dropped:
        // the kernel stays authoritative and packets keep flowing (plan 4.3).
        try { if (!this.disposed) await this.send(next.action, next.epoch) } catch { this.frame.dataset.mirrorRefusals = String(Number(this.frame.dataset.mirrorRefusals ?? 0) + 1) } finally { this.pendingInputs-- }
      }
    } finally { this.sending = false }
  }
  private scrolling(): boolean { return performance.now() - this.localScrollAt < 500 || this.queue.some(q => q.action.kind === 'scroll_to') }
  private idOf(node: Node | null | undefined): string | undefined {
    for (let n: Node | null | undefined = node, depth = 0; n && depth < 512; depth++) { const id = this.ids.get(n); if (id) return id; n = n.parentNode ?? (n as ShadowRoot).host }
    return undefined
  }
  private bind(doc: Document): void {
    if (this.bound.has(doc)) return
    this.bound.add(doc)
    const on = (type: string, fn: (event: Event) => void): void => doc.addEventListener(type, fn, { capture: true, passive: false })
    const target = (event: Event): Element | null => { const node = event.composedPath()[0] as Node; return node?.nodeType === 1 ? node as Element : node?.parentElement ?? null }
    const element = (event: Event): { id: string; el: Element } | null => {
      for (let el = target(event); el; el = el.parentElement ?? ((el.getRootNode() as ShadowRoot).host ?? null)) { const id = this.ids.get(el); if (id && this.records.get(id)?.kind !== 'text') return this.records.get(id)?.kind === 'mask' ? null : { id, el } }
      return null
    }
    const offset = (el: Element, event: MouseEvent): { x: number; y: number } => { const r = el.getBoundingClientRect(); return { x: event.clientX - r.left, y: event.clientY - r.top } }
    // A click that ends a text drag selects; it is not forwarded as a click.
    let origin: { x: number; y: number } | null = null, dragged = false
    on('pointerdown', event => { const e = event as PointerEvent; origin = { x: e.clientX, y: e.clientY }; dragged = false })
    on('pointermove', event => { const e = event as PointerEvent; if (origin && (e.buttons & 1) && Math.hypot(e.clientX - origin.x, e.clientY - origin.y) > 4) dragged = true })
    const selecting = (el: Element): boolean => { if (el.closest('input,textarea,button,select,[contenteditable]')) return false; const s = doc.getSelection(); return Boolean(s && !s.isCollapsed && (el.contains(s.anchorNode) || el.contains(s.focusNode))) }
    on('click', event => { event.preventDefault(); const hit = element(event); if (!hit || dragged || selecting(hit.el)) { dragged = false; return } this.enqueue({ kind: 'click', node_id: hit.id, ...offset(hit.el, event as MouseEvent) }) })
    on('auxclick', event => event.preventDefault())
    on('dragstart', event => event.preventDefault())
    on('submit', event => event.preventDefault())
    // Native, local scrolling; opaque regions (canvas, video, foreign stills) take the wheel themselves.
    on('wheel', event => { const hit = element(event), wheel = event as WheelEvent; if (!hit || this.records.get(hit.id)?.kind !== 'tile') { this.localScrollAt = performance.now(); return } event.preventDefault(); this.enqueue({ kind: 'scroll', node_id: hit.id, ...offset(hit.el, wheel), delta_x: Math.trunc(wheel.deltaX), delta_y: Math.trunc(wheel.deltaY) }) })
    on('scroll', this.scrolled)
    on('keydown', event => {
      const { key, shiftKey, ctrlKey, metaKey, altKey } = event as KeyboardEvent
      if (ctrlKey || metaKey || altKey) return
      // Page keys scroll the viewer's own copy natively; the scroll listener sends the position.
      if (['Tab', 'Enter', 'Escape', 'Backspace', 'Delete', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(key)) {
        event.preventDefault()
        // Shift keeps its meaning for focus and selection keys (back-tab, extending a range).
        this.enqueue({ kind: 'key', key: shiftKey && ['Tab', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(key) ? `Shift+${key}` : key })
      }
    })
    let composed: string | null = null
    on('beforeinput', event => { const input = event as InputEvent; event.preventDefault(); if (input.isComposing || input.inputType.includes('Composition') || input.data === composed || !input.data) return; this.enqueue({ kind: 'text', text: input.data }) })
    on('compositionend', event => { event.preventDefault(); const data = (event as CompositionEvent).data; if (data) { composed = data; setTimeout(() => { composed = null }, 0); this.enqueue({ kind: 'text', text: data }) } })
    on('selectionchange', () => { if (this.applying) return; const selection = doc.getSelection(); if (!selection || selection.isCollapsed) return; const a = this.ids.get(selection.anchorNode!), b = this.ids.get(selection.focusNode!); if (a && b && this.records.get(a)?.kind === 'text' && this.records.get(b)?.kind === 'text') this.enqueue({ kind: 'selection', anchor_id: a, anchor_offset: selection.anchorOffset, focus_id: b, focus_offset: selection.focusOffset }) })
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
    // SVG stays an image: data: URLs have an opaque origin if ever opened top-level.
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
    if (entry.kind === 'css') entry.node.textContent = this.substitute(entry.raw)
    else if (entry.kind === 'attr') entry.node.setAttribute('style', this.substitute(entry.raw))
    else {
      const { node: root, raw } = entry as Extract<Styled, { kind: 'adopted' }>
      const view = (root.nodeType === 9 ? root as Document : (root as ShadowRoot).ownerDocument).defaultView as (Window & typeof globalThis) | null
      if (view) root.adoptedStyleSheets = raw.map(text => { const sheet = new view.CSSStyleSheet(); sheet.replaceSync(this.substitute(text)); return sheet })
    }
  }
  private setStyled(id: string, entry: Styled | null): void {
    if (!entry) { this.styled.delete(id); return }
    entry.keys = entry.kind === 'adopted' ? entry.raw.flatMap(text => this.keysOf(text)) : this.keysOf(entry.raw)
    this.styled.set(entry.kind === 'adopted' ? `${id}#adopted` : entry.kind === 'attr' ? `${id}#style` : id, entry)
    this.restyle(entry)
  }
  // ---- DOM construction
  private create(record: Mirror2Record, doc: Document): Node {
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
      else try { element.setAttribute(name, value) } catch { /* invalid names were refused by validation */ }
    }
    if (record.kind === 'element' && record.tag === 'style') this.setStyled(record.id, { kind: 'css', raw: record.css ?? '', node: element, keys: [] })
    if (record.res !== undefined) this.image(record.id, element, record.res)
    if (record.kind === 'mask' || record.kind === 'tile') {
      const style = (element as HTMLElement).style
      const [width, height] = record.size ?? [0, 0]
      style.setProperty('box-sizing', 'border-box', 'important'); style.setProperty('width', `${width}px`, 'important'); style.setProperty('height', `${height}px`, 'important')
      style.setProperty('overflow', 'hidden', 'important'); style.setProperty('background-color', 'black', 'important'); style.setProperty('color', 'transparent', 'important')
      // A mask keeps the protected box: inline/contents boxes become inline blocks of the source size.
      if (record.kind === 'mask') { style.setProperty('display', record.display === 'none' ? 'none' : !record.display || ['inline', 'contents'].includes(record.display) ? 'inline-block' : record.display, 'important'); style.setProperty('background-image', 'none', 'important'); element.setAttribute('aria-label', 'Protected content') }
      if (record.kind === 'tile') { style.setProperty('display', record.display === 'none' ? 'none' : 'inline-block'); if (element.localName === 'iframe') element.setAttribute('sandbox', '') }
    }
    if (record.kind === 'frame') { element.setAttribute('sandbox', 'allow-same-origin'); element.setAttribute('referrerpolicy', 'no-referrer') }
    // A select's state waits for its options (the build appends children after their parent).
    if (record.form) { if (element.localName === 'select') this.selects.push([element, record.form]); else this.form(element, record.form) }
  }
  private image(id: string, element: Element, key: string | null): void {
    const url = key ? this.resources.get(key) ?? null : null
    if (element.localName === 'img') { if (url) element.setAttribute('src', url); else element.removeAttribute('src') }
    else if (element.namespaceURI === NS.svg) { if (url) element.setAttribute('href', url); else element.removeAttribute('href') }
    const record = this.records.get(id); if (record) { if (key) record.res = key; else delete record.res }
  }
  private form(element: Element, form: Mirror2Form): void {
    const field = element as HTMLInputElement
    if (field.value !== form.value) field.value = form.value
    if ('checked' in field && field.type !== undefined) field.checked = form.checked
    if (element.localName === 'select') (element as unknown as HTMLSelectElement).selectedIndex = form.selected_index
    if ((field.getRootNode() as Document | ShadowRoot).activeElement === field) this.caret(field, form)
  }
  private caret(field: HTMLInputElement, form: Mirror2Form): void {
    if (form.selection_start !== null && form.selection_end !== null) try { field.setSelectionRange(form.selection_start, form.selection_end) } catch { /* not a text control */ }
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
    if (!root) try { root = host.attachShadow({ mode: 'open' }); root.addEventListener('scroll', this.scrolled, { capture: true, passive: true }) } catch { root = null }
    if (!root) { this.forget(record.id); return }
    root.replaceChildren(...Array.from(fragment.childNodes))
    this.dom.set(record.id, root); this.ids.set(root, record.id)
    if (record.adopted?.length) this.setStyled(record.id, { kind: 'adopted', raw: record.adopted as string[], node: root, keys: [] })
  }
  private forget(id: string): void {
    const node = this.dom.get(id), kind = this.records.get(id)?.kind
    if (kind === 'tile' || kind === 'mask') this.counts[kind]--
    this.dom.delete(id); this.records.delete(id); this.styled.delete(id); this.styled.delete(`${id}#style`); this.styled.delete(`${id}#adopted`)
    const tile = this.tileUrls.get(id); if (tile) { URL.revokeObjectURL(tile); this.tileUrls.delete(id) }
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
      this.records.set(documentRecord.id, documentRecord); this.dom.set(documentRecord.id, nested); this.ids.set(nested, documentRecord.id)
      const built = this.build(subtree.slice(1), nested, frames, scrolls)
      const html = subtree[1] ? built.get(subtree[1].id) as Element | undefined : undefined
      if (html) { if (nested.documentElement) nested.documentElement.replaceWith(html); else nested.appendChild(html); this.csp(nested) }
      if (documentRecord.adopted?.length) this.setStyled(documentRecord.id, { kind: 'adopted', raw: documentRecord.adopted as string[], node: nested, keys: [] })
      this.bind(nested)
    }
  }
  private subtree(records: Mirror2Record[], rootId: string): Mirror2Record[] {
    const ids = new Set([rootId]), out: Mirror2Record[] = []
    for (const record of records) if (record.id === rootId || record.parent !== null && ids.has(record.parent)) { ids.add(record.id); out.push(record) }
    return out
  }
  private csp(doc: Document): void {
    const head = doc.head ?? doc.documentElement.insertBefore(doc.createElement('head'), doc.documentElement.firstChild)
    const meta = doc.createElement('meta'); meta.httpEquiv = 'Content-Security-Policy'; meta.content = mirror2SandboxCsp; head.prepend(meta)
  }
  private sheets = new Map<string, string>()
  async apply(wire: Mirror2Packet): Promise<void> {
    const started = performance.now()
    if (this.disposed || !this.doc) throw Error('MP-08: mirror unavailable')
    if (!wire.reset && (wire.base_sequence !== this.sequence || wire.document_id !== this.documentId)) throw Error('MP-11: mirror lost base')
    const packet = resolveMirror2Sheets(wire, this.sheets)
    validateMirror2Packet(packet, this.records)
    // Resource keys are per document; a new document starts an empty map. A
    // reset restarts resources still in slices (the kernel sends them again).
    if (packet.reset && packet.document_id !== this.documentId) { for (const url of this.resources.values()) if (url.startsWith('blob:')) URL.revokeObjectURL(url); this.resources.clear() }
    if (packet.reset) this.slices.clear()
    const complete: Mirror2Resource[] = []
    for (const slice of packet.resources) { const resource = this.assemble(slice); if (resource) { await this.addResource(resource); complete.push(resource) } }
    if (this.disposed || !this.doc) throw Error('MP-08: mirror closed during validation')
    this.applying = true
    const scrolls: Array<[Element, number, number]> = []
    try {
      if (packet.reset) this.reset(packet, scrolls)
      for (const op of packet.ops ?? []) this.op(op, scrolls)
      for (const resource of complete) for (const entry of this.styled.values()) if (entry.keys.includes(resource.key)) this.restyle(entry)
      for (const resource of complete) for (const [id, record] of this.records) if (record.res === resource.key) { const node = this.dom.get(id); if (node?.nodeType === 1) this.image(id, node as Element, record.res) }
      for (const tile of packet.tiles) this.tile(tile)
      // The viewer owns scroll while it scrolls; the kernel's positions (the latest per
      // scroller, kept while deferred) apply on reset or once the viewer settles.
      if (packet.reset) this.deferred.clear()
      for (const [element, x, y] of scrolls) this.deferred.set(element, [x, y])
      if (packet.reset || !this.scrolling()) {
        const pending = this.deferred; this.deferred = new Map()
        for (const [element, [x, y]] of pending as Map<Element, [number, number]>) {
          // A frame document scrolls its viewport (its root element: a quirks-mode viewer document would not).
          const doc = (element as Node).nodeType === 9 ? element as unknown as Document : element === element.ownerDocument.documentElement ? element.ownerDocument : null
          if (doc) { const view = doc.defaultView; view?.scrollTo(x, y); if (view) this.applied.set(doc, [view.scrollX, view.scrollY]); continue }
          if (element.scrollLeft !== x) element.scrollLeft = x; if (element.scrollTop !== y) element.scrollTop = y
          this.applied.set(element, [element.scrollLeft, element.scrollTop])
        }
        const view = this.frame.contentWindow!; view.scrollTo(packet.scroll[0], packet.scroll[1]); this.applied.set(this.doc, [view.scrollX, view.scrollY])
      }
      this.sequence = packet.sequence; this.documentId = packet.document_id
      if (!this.pendingInputs) {
        const focused = packet.focused ? this.dom.get(packet.focused) as HTMLElement | undefined : undefined
        // A control built while detached could not take its caret: restore it once focused.
        if (focused && this.active() !== focused) { focused.focus?.({ preventScroll: true }); const form = this.records.get(packet.focused!)?.form; if (form) this.caret(focused as HTMLInputElement, form) }
        if (packet.selection) { const s = packet.selection, a = this.dom.get(s.anchor_id), b = this.dom.get(s.focus_id); if (a && b) a.ownerDocument?.getSelection()?.setBaseAndExtent(a, s.anchor_offset, b, s.focus_offset) }
      }
    } finally { this.applying = false }
    Object.assign(this.frame.dataset, { mirrorSequence: String(packet.sequence), mirrorRegions: String(this.counts.tile), mirrorMasks: String(this.counts.mask) })
    this.timed(packet.reset ? 'apply_reset' : 'apply_delta', started)
  }
  private reset(packet: Mirror2Packet, scrolls: Array<[Element, number, number]>): void {
    const doc = this.doc!
    for (const url of this.tileUrls.values()) URL.revokeObjectURL(url)
    this.tileUrls.clear(); this.dom.clear(); this.records.clear(); this.styled.clear(); this.ids = new WeakMap(); this.counts = { tile: 0, mask: 0 }
    const records = packet.nodes ?? [], frames: Array<{ frame: HTMLIFrameElement; id: string }> = []
    const root = records[0]!
    const main = this.subtree(records, root.id)
    // The top document record maps onto the sandbox document itself.
    this.records.set(root.id, root); this.dom.set(root.id, doc); this.ids.set(doc, root.id)
    const built = this.build(main.slice(1), doc, frames, scrolls)
    const html = main[1] ? built.get(main[1].id) as Element | undefined : undefined
    if (html) { doc.documentElement.replaceWith(html); this.csp(doc) }
    if (root.adopted?.length) this.setStyled(root.id, { kind: 'adopted', raw: root.adopted as string[], node: doc, keys: [] })
    else doc.adoptedStyleSheets = []
    this.hydrateFrames(frames, records, scrolls)
  }
  private op(op: Mirror2Op, scrolls: Array<[Element, number, number]>): void {
    const node = this.dom.get(op.id)
    if (!node) return // removed earlier in this packet
    switch (op.op) {
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
        const keep = new Set(desired)
        // Forgotten before removal: a detached iframe no longer exposes its document.
        for (const child of Array.from(container.childNodes)) if (!keep.has(child) && (this.ids.has(child) || child.nodeType !== 1 || !(child as Element).matches?.('meta[http-equiv]'))) { this.forgetTree(child); container.removeChild(child) }
        for (let i = 0; i < desired.length; i++) if (container.childNodes[i] !== desired[i]) container.insertBefore(desired[i]!, container.childNodes[i] ?? null)
        for (const id of shadowIds) { const record = this.records.get(id)!; const fragment = fresh.get(id); if (fragment && node.nodeType === 1) this.attachShadow(node as Element, record, fragment as DocumentFragment) }
        if (node.nodeType === 9 && !(node as Document).head?.querySelector('meta[http-equiv]')) this.csp(node as Document)
        this.hydrateFrames(frames, op.nodes, scrolls)
        break
      }
      case 'attr': {
        const element = node as Element; const record = this.records.get(op.id)!
        record.attrs ??= {}
        if (op.value === null) { delete record.attrs[op.name]; if (op.name === 'style') this.styled.delete(`${op.id}#style`); element.removeAttribute(op.name) }
        else { record.attrs[op.name] = op.value; if (op.name === 'style') this.setStyled(op.id, { kind: 'attr', raw: op.value, node: element, keys: [] }); else element.setAttribute(op.name, op.value) }
        break
      }
      case 'text': node.textContent = op.text; this.records.get(op.id)!.text = op.text; break
      case 'css': { this.records.get(op.id)!.css = op.css; this.setStyled(op.id, { kind: 'css', raw: op.css, node: node as Element, keys: [] }); break }
      case 'adopted': this.setStyled(op.id, { kind: 'adopted', raw: op.sheets as string[], node: node as Document | ShadowRoot, keys: [] }); break
      case 'form': { const record = this.records.get(op.id)!, form = record.form = { value: '', checked: false, selected_index: -1, selection_start: null, selection_end: null, ...record.form, ...op.form }; this.form(node as Element, form); break }
      case 'scroll': scrolls.push([node as Element, op.scroll[0], op.scroll[1]]); break
      case 'size': { const style = (node as HTMLElement).style; style.setProperty('width', `${op.size[0]}px`, 'important'); style.setProperty('height', `${op.size[1]}px`, 'important'); this.records.get(op.id)!.size = op.size; break }
      case 'res': this.image(op.id, node as Element, op.res); break
    }
  }
  // Opaque region pixels paint as the element's own background: they scroll and
  // reflow with the mirrored layout instead of floating at a stale viewport box.
  private tile(tile: Mirror2Tile): void {
    const element = this.dom.get(tile.node_id) as HTMLElement | undefined; if (!element) return
    const old = this.tileUrls.get(tile.node_id); if (old) URL.revokeObjectURL(old)
    const url = URL.createObjectURL(new Blob([bytesOf(tile.data_base64) as Uint8Array<ArrayBuffer>], { type: 'image/png' })); this.tileUrls.set(tile.node_id, url)
    const style = element.style
    style.setProperty('background-image', `url("${url}")`, 'important'); style.setProperty('background-repeat', 'no-repeat', 'important')
    style.setProperty('background-position', `${tile.x}px ${tile.y}px`, 'important'); style.setProperty('background-size', `${tile.width}px ${tile.height}px`, 'important')
  }
  close(): void {
    this.disposed = true; this.doc = null
    for (const url of this.resources.values()) if (url.startsWith('blob:')) URL.revokeObjectURL(url)
    for (const url of this.tileUrls.values()) URL.revokeObjectURL(url)
    URL.revokeObjectURL(this.empty); this.deferred.clear(); this.resources.clear(); this.slices.clear(); this.tileUrls.clear(); this.dom.clear(); this.records.clear(); this.styled.clear(); this.frame.remove()
  }
}

export interface Mirror2Transport { protocolVersion: number; request(request: unknown): Promise<unknown> }
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
type Binding = { tab_id: string; generation: number; device_scale_factor: 1 | 2 }
const GAP_MS = 500
// Credit-gated push: `credits` long-poll requests stay outstanding; packets
// apply strictly in sequence. A gap or failed credit asks for a fresh snapshot.
type Renderer = Pick<BrowserMirror2Renderer, 'ready' | 'apply' | 'close' | 'frame'>
// A failure streak that outlives this with no packet is terminal (the runtime then
// shows protected video and retries later); shorter streaks back off.
const FAILING_MS = 15_000
export async function attachBrowserMirror2(transport: Mirror2Transport, container: HTMLElement, binding: Binding, handlers: { failure(error: unknown): void; packet?(packet: Mirror2Packet): void }, { credits = 4, waitMs = 2000, failingMs = FAILING_MS, renderer: createRenderer = (container: HTMLElement, send: ConstructorParameters<typeof BrowserMirror2Renderer>[1]): Renderer => new BrowserMirror2Renderer(container, send) } = {}) {
  if (!Number.isInteger(transport.protocolVersion) || transport.protocolVersion < browserMirror2MinimumProtocolVersion) throw Error('MP-08: DOM mirror v2 requires protocol 489')
  const request = async (command: unknown): Promise<any> => { const response = await transport.request({ KernelBrowser: { command } }) as { KernelBrowser?: { result?: unknown } }; if (!response.KernelBrowser?.result) throw Error('MP-08: invalid mirror response'); return response.KernelBrowser.result }
  const subscribed = await request({ op: 'mirror_subscribe', ...binding, wire: 2 }); const subscription_id = subscribed.subscription_id as string
  // Four credits while the page or the viewer is active, one when idle (one heartbeat per wait).
  let closed = false, inflight = 0, applied = 0, wantReset = true, resetOutstanding = false, activeAt = -Infinity, gapSince = 0, failures = 0, failingSince = 0
  const buffered = new Map<number, Mirror2WirePacket>()
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
    const header = (name: 'scroll' | 'focused' | 'selection') => Object.hasOwn(body, name) ? body[name] : base![name]
    return decodeMirror2Packet({ ...body, ...media, subscription_id, tab_id: binding.tab_id, generation: binding.generation, document_id: base.document_id, base_sequence: wire.sequence - 1, reset: false,
      css_width: base.css_width, css_height: base.css_height, device_scale_factor: base.device_scale_factor, scroll: header('scroll'), focused: header('focused'), selection: header('selection') } as Mirror2Packet)
  }
  const renderer = createRenderer(container, (action, epoch) => { activeAt = performance.now(); fill(); return request({ op: 'mirror_input', tab_id: binding.tab_id, generation: binding.generation, document_id: epoch.document_id, subscription_id, sequence: epoch.sequence, action }) })
  try { await renderer.ready() } catch (error) { renderer.close(); await request({ op: 'mirror_close', subscription_id, generation: binding.generation }).catch(() => {}); throw error }
  let chain: Promise<void> = Promise.resolve()
  // Packets apply one at a time. A failed application rejects only its own
  // caller (which asks for a reset); later packets still drain.
  const drain = (): Promise<void> => { const run = chain.then(async () => {
    for (let wire = [...buffered.values()].find(p => p.reset || p.sequence === applied + 1); wire; wire = [...buffered.values()].find(p => p.reset || p.sequence === applied + 1)) {
      for (const seq of buffered.keys()) if (seq <= wire.sequence) buffered.delete(seq)
      const next = await expand(wire)
      if (!next.fallback) await renderer.apply(next)
      // Only an applied packet ends a failure streak (a reply that cannot be applied does not).
      base = next; failures = 0; failingSince = 0
      if (next.reset || next.ops?.length || next.resources.length || next.tiles.length) activeAt = performance.now()
      applied = next.sequence
      if (next.reset) wantReset = false
      handlers.packet?.(next)
    }
    if (buffered.size > 8) { buffered.clear(); wantReset = true }
    // Credits and replies may overtake each other by milliseconds; a gap that
    // outlives that is permanent (a reply lost with a dropped socket): reset.
    if (!buffered.size) gapSince = 0
    else if (!gapSince) { const since = gapSince = performance.now(); setTimeout(() => { if (gapSince === since && buffered.size && !closed) { buffered.clear(); gapSince = 0; wantReset = true; fill() } }, GAP_MS) }
  }); chain = run.catch(() => { buffered.clear(); gapSince = 0 }); return run }
  // A retired subscription or generation never recovers by itself.
  // Stopping local credits and releasing the kernel subscription are separate: the latter happens once.
  let remoteClosed: Promise<void> | null = null
  const closeRemote = (): Promise<void> => remoteClosed ??= request({ op: 'mirror_close', subscription_id, generation: binding.generation }).then(() => {}, () => {})
  const terminal = (error: unknown): boolean => error instanceof Error && /stale or foreign mirror/.test(error.message)
  const fatal = (error: unknown): boolean => error instanceof Error && /MP-11: (invalid|unsafe|foreign|executable|active|protected|mirror resource digest|mirror packet bounds)/.test(error.message)
  const credit = (): void => {
    const reset = wantReset && !resetOutstanding
    if (reset) resetOutstanding = true
    inflight++
    request({ op: 'mirror_next', subscription_id, generation: binding.generation, after_sequence: reset ? 0 : applied, drift_nodes: [], wait_ms: waitMs })
      .then(async (packet: Mirror2WirePacket) => {
        if (!packet || packet.wire !== 2 || !Number.isSafeInteger(packet.sequence) || packet.sequence <= 0) throw Error('MP-11: foreign mirror packet')
        if (packet.reset) resetOutstanding = false
        // A replayed or late copy of an applied packet is not a gap.
        if (packet.sequence > applied) buffered.set(packet.sequence, packet)
        await drain()
      })
      .catch(error => {
        if (reset) resetOutstanding = false; wantReset = true; failures++; failingSince ||= performance.now()
        if (!closed && (fatal(error) || terminal(error) || performance.now() - failingSince > failingMs)) { closed = true; inflater?.close(); renderer.close(); handlers.failure(error); void closeRemote() }
      })
      .finally(() => { inflight--; if (!closed) setTimeout(fill, failures ? Math.min(4000, 250 * 2 ** (failures - 1)) : 0) })
  }
  // Pipelined credits start once a snapshot is applied; a reset is a single
  // credit and never waits behind credits in flight (the kernel ends their wait).
  const fill = (): void => {
    if (!closed && applied && wantReset && !resetOutstanding) credit()
    while (!closed && inflight < (wantReset || !applied || performance.now() - activeAt > 3000 ? 1 : credits)) credit()
  }
  fill()
  return {
    renderer,
    async close() { if (!closed) { closed = true; inflater?.close(); renderer.close() } await closeRemote() },
    takeover: () => request({ op: 'display_takeover', tab_id: binding.tab_id, generation: binding.generation }),
    release: () => request({ op: 'display_release', tab_id: binding.tab_id, generation: binding.generation }),
  }
}
