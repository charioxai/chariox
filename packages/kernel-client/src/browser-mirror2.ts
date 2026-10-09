// MP-08/MP-10/MP-11: DOM mirror v2 renderer (local protocol 482). The page's own
// sanitized stylesheets and attributes are rebuilt in a script-free sandbox;
// deltas apply in place. No origin I/O: resources arrive as kernel bytes.
import { validateMirror2Packet, decodeMirror2Packet, mirror2SandboxCsp } from './browser-mirror2-security.js'
import type { Mirror2Action, Mirror2Op, Mirror2Packet, Mirror2Record, Mirror2Resource, Mirror2Tile } from './browser-mirror2-types.js'
export * from './browser-mirror2-types.js'
export { mirror2SandboxCsp, validateMirror2Packet, decodeMirror2Packet } from './browser-mirror2-security.js'
export const browserMirror2MinimumProtocolVersion = 482
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
  private styled = new Map<string, Styled>()
  private tileUrls = new Map<string, string>()
  private bound = new Set<Document>()
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
  constructor(private container: HTMLElement, private send: (action: Mirror2Action, epoch: { sequence: number; document_id: string }) => Promise<unknown>, private failure: (error: unknown) => void) {
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
        try { if (!this.disposed) await this.send(next.action, next.epoch) } catch (error) { this.failure(error) } finally { this.pendingInputs-- }
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
    let scrolled = new Set<Node>(), frame = 0
    const position = (target: Node): [number, number] => target.nodeType === 9 ? [(target as Document).defaultView?.scrollX ?? 0, (target as Document).defaultView?.scrollY ?? 0] : [(target as Element).scrollLeft, (target as Element).scrollTop]
    on('scroll', event => {
      const target = event.target as Node, kernel = this.applied.get(target), now = position(target)
      if (this.applying || kernel && Math.abs(kernel[0] - now[0]) < 1 && Math.abs(kernel[1] - now[1]) < 1) return
      this.applied.delete(target)
      this.localScrollAt = performance.now(); scrolled.add(target)
      frame ||= requestAnimationFrame(() => {
        frame = 0; const targets = scrolled; scrolled = new Set()
        for (const target of targets) {
          const doc = target.nodeType === 9 ? target as Document : null, id = this.ids.get(target)
          if (doc) this.enqueue({ kind: 'scroll_to', node_id: doc === this.doc ? null : id ?? null, x: doc.defaultView?.scrollX ?? 0, y: doc.defaultView?.scrollY ?? 0 })
          else if (id) this.enqueue({ kind: 'scroll_to', node_id: id, x: (target as Element).scrollLeft, y: (target as Element).scrollTop })
        }
      })
    })
    on('keydown', event => {
      const { key, shiftKey, ctrlKey, metaKey, altKey } = event as KeyboardEvent
      if (ctrlKey || metaKey || altKey) return
      if (['Tab', 'Enter', 'Escape', 'Backspace', 'Delete', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'PageUp', 'PageDown'].includes(key)) {
        event.preventDefault(); if (key === 'PageUp' || key === 'PageDown') return
        this.enqueue({ kind: 'key', key: key === 'Tab' && shiftKey ? 'Shift+Tab' : key })
      }
    })
    let composed: string | null = null
    on('beforeinput', event => { const input = event as InputEvent; event.preventDefault(); if (input.isComposing || input.inputType.includes('Composition') || input.data === composed || !input.data) return; this.enqueue({ kind: 'text', text: input.data }) })
    on('compositionend', event => { event.preventDefault(); const data = (event as CompositionEvent).data; if (data) { composed = data; setTimeout(() => { composed = null }, 0); this.enqueue({ kind: 'text', text: data }) } })
    on('selectionchange', () => { if (this.applying) return; const selection = doc.getSelection(); if (!selection || selection.isCollapsed) return; const a = this.ids.get(selection.anchorNode!), b = this.ids.get(selection.focusNode!); if (a && b && this.records.get(a)?.kind === 'text' && this.records.get(b)?.kind === 'text') this.enqueue({ kind: 'selection', anchor_id: a, anchor_offset: selection.anchorOffset, focus_id: b, focus_offset: selection.focusOffset }) })
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
      if (record.kind === 'mask') { if (!record.display || record.display === 'inline') style.setProperty('display', 'inline-block', 'important'); style.setProperty('background-image', 'none', 'important'); element.setAttribute('aria-label', 'Protected content') }
      if (record.kind === 'tile') { style.setProperty('display', record.display === 'none' ? 'none' : 'inline-block'); if (element.localName === 'iframe') element.setAttribute('sandbox', '') }
    }
    if (record.kind === 'frame') { element.setAttribute('sandbox', 'allow-same-origin'); element.setAttribute('referrerpolicy', 'no-referrer') }
    if (record.form) this.form(element, record.form)
  }
  private image(id: string, element: Element, key: string | null): void {
    const url = key ? this.resources.get(key) ?? null : null
    if (element.localName === 'img') { if (url) element.setAttribute('src', url); else element.removeAttribute('src') }
    else if (element.namespaceURI === NS.svg) { if (url) element.setAttribute('href', url); else element.removeAttribute('href') }
    const record = this.records.get(id); if (record) { if (key) record.res = key; else delete record.res }
  }
  private form(element: Element, form: NonNullable<Mirror2Record['form']>): void {
    const field = element as HTMLInputElement
    if (field.value !== form.value) field.value = form.value
    if ('checked' in field && field.type !== undefined) field.checked = form.checked
    if (element.localName === 'select') (element as unknown as HTMLSelectElement).selectedIndex = form.selected_index
    if (form.selection_start !== null && form.selection_end !== null && field.ownerDocument.activeElement === field) try { field.setSelectionRange(form.selection_start, form.selection_end) } catch { /* not a text control */ }
  }
  // Records are pre-order; each record's children follow it in list order.
  // Frame documents are built separately into the frame's own document.
  private build(records: Mirror2Record[], doc: Document, frames: Array<{ frame: HTMLIFrameElement; id: string }>, scrolls: Array<[Element, number, number]>): Map<string, Node> {
    const built = new Map<string, Node>(), shadows: Array<[Element, Mirror2Record]> = [], skipped = new Set<string>()
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
    return built
  }
  private attachShadow(host: Element, record: Mirror2Record, fragment: DocumentFragment): void {
    let root: ShadowRoot | null = host.shadowRoot
    if (!root) try { root = host.attachShadow({ mode: 'open' }) } catch { root = null }
    if (!root) { this.forget(record.id); return }
    root.replaceChildren(...Array.from(fragment.childNodes))
    this.dom.set(record.id, root); this.ids.set(root, record.id)
    if (record.adopted?.length) this.setStyled(record.id, { kind: 'adopted', raw: record.adopted, node: root, keys: [] })
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
      if (documentRecord.adopted?.length) this.setStyled(documentRecord.id, { kind: 'adopted', raw: documentRecord.adopted, node: nested, keys: [] })
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
  async apply(packet: Mirror2Packet): Promise<void> {
    const started = performance.now()
    if (this.disposed || !this.doc) throw Error('MP-08: mirror unavailable')
    if (!packet.reset && (packet.base_sequence !== this.sequence || packet.document_id !== this.documentId)) throw Error('MP-11: mirror lost base')
    validateMirror2Packet(packet, this.records)
    // Resource keys are per document; a new document starts an empty map.
    if (packet.reset && packet.document_id !== this.documentId) { for (const url of this.resources.values()) if (url.startsWith('blob:')) URL.revokeObjectURL(url); this.resources.clear() }
    for (const resource of packet.resources) await this.addResource(resource)
    if (this.disposed || !this.doc) throw Error('MP-08: mirror closed during validation')
    this.applying = true
    const scrolls: Array<[Element, number, number]> = []
    try {
      if (packet.reset) this.reset(packet, scrolls)
      for (const op of packet.ops ?? []) this.op(op, scrolls)
      for (const resource of packet.resources) for (const entry of this.styled.values()) if (entry.keys.includes(resource.key)) this.restyle(entry)
      for (const resource of packet.resources) for (const [id, record] of this.records) if (record.res === resource.key) { const node = this.dom.get(id); if (node?.nodeType === 1) this.image(id, node as Element, record.res) }
      for (const tile of packet.tiles) this.tile(tile)
      // The viewer owns scroll while it scrolls; the kernel's position applies on reset or when settled.
      if (packet.reset || !this.scrolling()) {
        for (const [element, x, y] of scrolls) {
          if ((element as Node).nodeType === 9) { const view = (element as unknown as Document).defaultView; view?.scrollTo(x, y); if (view) this.applied.set(element, [view.scrollX, view.scrollY]); continue }
          if (element.scrollLeft !== x) element.scrollLeft = x; if (element.scrollTop !== y) element.scrollTop = y
          this.applied.set(element, [element.scrollLeft, element.scrollTop])
        }
        const view = this.frame.contentWindow!; view.scrollTo(packet.scroll[0], packet.scroll[1]); this.applied.set(this.doc, [view.scrollX, view.scrollY])
      }
      this.sequence = packet.sequence; this.documentId = packet.document_id
      if (!this.pendingInputs) {
        const focused = packet.focused ? this.dom.get(packet.focused) as HTMLElement | undefined : undefined
        if (focused && this.doc.activeElement !== focused) focused.focus?.({ preventScroll: true })
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
    if (root.adopted?.length) this.setStyled(root.id, { kind: 'adopted', raw: root.adopted, node: doc, keys: [] })
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
          if (nested?.documentElement) this.forgetTree(nested.documentElement)
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
        for (const child of Array.from(container.childNodes)) if (!keep.has(child) && (this.ids.has(child) || child.nodeType !== 1 || !(child as Element).matches?.('meta[http-equiv]'))) { container.removeChild(child); if (!this.isAttached(child)) this.forgetTree(child) }
        for (let i = 0; i < desired.length; i++) if (container.childNodes[i] !== desired[i]) container.insertBefore(desired[i]!, container.childNodes[i] ?? null)
        for (const id of shadowIds) { const record = this.records.get(id)!; const fragment = fresh.get(id); if (fragment && node.nodeType === 1) this.attachShadow(node as Element, record, fragment as DocumentFragment) }
        if (node.nodeType === 9 && !(node as Document).head?.querySelector('meta[http-equiv]')) this.csp(node as Document)
        this.hydrateFrames(frames, op.nodes, scrolls)
        break
      }
      case 'attr': {
        const element = node as Element; const record = this.records.get(op.id)!
        record.attrs ??= {}
        if (op.value === null) { delete record.attrs[op.name]; if (op.name === 'style') this.setStyled(op.id, null); element.removeAttribute(op.name) }
        else { record.attrs[op.name] = op.value; if (op.name === 'style') this.setStyled(op.id, { kind: 'attr', raw: op.value, node: element, keys: [] }); else element.setAttribute(op.name, op.value) }
        break
      }
      case 'text': node.textContent = op.text; this.records.get(op.id)!.text = op.text; break
      case 'css': { this.records.get(op.id)!.css = op.css; this.setStyled(op.id, { kind: 'css', raw: op.css, node: node as Element, keys: [] }); break }
      case 'adopted': this.setStyled(op.id, { kind: 'adopted', raw: op.sheets, node: node as Document | ShadowRoot, keys: [] }); break
      case 'form': this.records.get(op.id)!.form = op.form; this.form(node as Element, op.form); break
      case 'scroll': scrolls.push([node as Element, op.scroll[0], op.scroll[1]]); break
      case 'size': { const style = (node as HTMLElement).style; style.setProperty('width', `${op.size[0]}px`, 'important'); style.setProperty('height', `${op.size[1]}px`, 'important'); this.records.get(op.id)!.size = op.size; break }
      case 'res': this.image(op.id, node as Element, op.res); break
    }
  }
  private isAttached(node: Node): boolean { return node.isConnected || node.parentNode !== null }
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
    URL.revokeObjectURL(this.empty); this.resources.clear(); this.tileUrls.clear(); this.dom.clear(); this.records.clear(); this.styled.clear(); this.frame.remove()
  }
}

export interface Mirror2Transport { protocolVersion: number; request(request: unknown): Promise<unknown> }
// The kernel gzips the scrubbed packet body (no relay compression); encoded
// resource/region bytes travel beside it. Inflation is bounded before parsing.
export async function inflateMirror2Packet(wire: { encoding?: string; packet_base64?: string; packet_bytes?: number; resources?: unknown; tiles?: unknown } & Partial<Mirror2Packet>): Promise<Mirror2Packet> {
  if (wire.encoding === undefined) return wire as Mirror2Packet
  if (wire.encoding !== 'gzip' || typeof wire.packet_base64 !== 'string' || !Number.isSafeInteger(wire.packet_bytes) || wire.packet_bytes! < 2 || wire.packet_bytes! > 256 * 1024 * 1024) throw Error('MP-11: mirror packet bounds')
  const reader = new Blob([bytesOf(wire.packet_base64) as Uint8Array<ArrayBuffer>]).stream().pipeThrough(new DecompressionStream('gzip')).getReader()
  const body = new Uint8Array(wire.packet_bytes!); let at = 0
  try { for (;;) { const { done, value } = await reader.read(); if (done) break; if (value.length > body.length - at) throw Error('MP-11: mirror packet bounds'); body.set(value, at); at += value.length } } finally { await reader.cancel().catch(() => {}) }
  if (at !== body.length) throw Error('MP-11: mirror packet bounds')
  const packet = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(body)) as Mirror2Packet
  return { ...packet, resources: wire.resources as Mirror2Resource[], tiles: wire.tiles as Mirror2Tile[] }
}
type Binding = { tab_id: string; generation: number; device_scale_factor: 1 | 2 }
// Credit-gated push: `credits` long-poll requests stay outstanding; packets
// apply strictly in sequence. A gap or failed credit asks for a fresh snapshot.
export async function attachBrowserMirror2(transport: Mirror2Transport, container: HTMLElement, binding: Binding, handlers: { failure(error: unknown): void; packet?(packet: Mirror2Packet): void }, { credits = 2, waitMs = 1500 } = {}) {
  if (!Number.isInteger(transport.protocolVersion) || transport.protocolVersion < browserMirror2MinimumProtocolVersion) throw Error('MP-08: DOM mirror v2 requires protocol 482')
  const request = async (command: unknown): Promise<any> => { const response = await transport.request({ KernelBrowser: { command } }) as { KernelBrowser?: { result?: unknown } }; if (!response.KernelBrowser?.result) throw Error('MP-08: invalid mirror response'); return response.KernelBrowser.result }
  const subscribed = await request({ op: 'mirror_subscribe', ...binding, wire: 2 }); const subscription_id = subscribed.subscription_id as string
  let closed = false, inflight = 0, applied = 0, wantReset = true, resetOutstanding = false
  const buffered = new Map<number, Mirror2Packet>()
  const renderer = new BrowserMirror2Renderer(container, (action, epoch) => request({ op: 'mirror_input', tab_id: binding.tab_id, generation: binding.generation, document_id: epoch.document_id, subscription_id, sequence: epoch.sequence, action }), handlers.failure)
  try { await renderer.ready() } catch (error) { renderer.close(); await request({ op: 'mirror_close', subscription_id, generation: binding.generation }).catch(() => {}); throw error }
  let chain: Promise<void> = Promise.resolve()
  const drain = (): Promise<void> => chain = chain.then(async () => {
    for (let next = [...buffered.values()].find(p => p.reset || p.base_sequence === applied); next; next = [...buffered.values()].find(p => p.reset || p.base_sequence === applied)) {
      for (const seq of buffered.keys()) if (seq <= next.sequence) buffered.delete(seq)
      if (!next.fallback) await renderer.apply(next)
      applied = next.sequence
      if (next.reset) wantReset = false
      handlers.packet?.(next)
    }
    if (buffered.size > 8) { buffered.clear(); wantReset = true }
  })
  const fatal = (error: unknown): boolean => error instanceof Error && /MP-11: (invalid|unsafe|foreign|executable|active|protected|mirror resource digest|mirror packet bounds)/.test(error.message)
  const credit = (): void => {
    const reset = wantReset && !resetOutstanding
    if (reset) resetOutstanding = true
    inflight++
    request({ op: 'mirror_next', subscription_id, generation: binding.generation, after_sequence: reset ? 0 : applied, drift_nodes: [], wait_ms: waitMs })
      .then(inflateMirror2Packet).then(decodeMirror2Packet).then(async (packet: Mirror2Packet) => {
        if (packet.subscription_id !== subscription_id || packet.tab_id !== binding.tab_id || packet.generation !== binding.generation || packet.wire !== 2) throw Error('MP-11: foreign mirror packet')
        if (packet.reset) resetOutstanding = false
        buffered.set(packet.sequence, packet); await drain()
      })
      .catch(error => { if (reset) resetOutstanding = false; wantReset = true; if (!closed && fatal(error)) { closed = true; renderer.close(); handlers.failure(error) } })
      .finally(() => { inflight--; if (!closed) setTimeout(fill, 0) })
  }
  // Pipelined credits start once a snapshot is applied; a reset is a single credit.
  const fill = (): void => { while (!closed && inflight < (wantReset || !applied ? 1 : credits)) credit() }
  fill()
  return {
    renderer,
    async close() { if (closed) return; closed = true; renderer.close(); await request({ op: 'mirror_close', subscription_id, generation: binding.generation }).catch(() => {}) },
    takeover: () => request({ op: 'display_takeover', tab_id: binding.tab_id, generation: binding.generation }),
    release: () => request({ op: 'display_release', tab_id: binding.tab_id, generation: binding.generation }),
  }
}
