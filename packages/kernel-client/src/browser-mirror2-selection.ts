// MP-08/MP-10/MP-11: reset IDs are epoch-local. Reconcile only an unchanged
// visible text range by structure, never by an old ID or a saved native input.
type Endpoint = { path: number[]; text: string; offset: number }
export type Mirror2Range = { anchor: Endpoint; focus: Endpoint; text: string }
export function rememberMirror2Range(doc: Document, indexed: (node: Node) => boolean): Mirror2Range | null {
  const selection = doc.getSelection()
  if (!selection || selection.isCollapsed) return null
  const endpoint = (node: Node | null, offset: number): Endpoint | null => {
    if (!node || node.nodeType !== 3 || !indexed(node)) return null
    const text = node.textContent ?? '', path: number[] = []
    for (let current: Node = node; current !== doc.documentElement;) {
      const parent = current.parentNode
      // Child documents and shadow roots reconcile through their own native
      // packets, not a positional guess across a frame or shadow boundary.
      if (!parent || path.length >= 512) return null
      const index = [...parent.childNodes].filter(indexed).findIndex(child => child === current)
      if (index < 0) return null
      path.unshift(index); current = parent
    }
    return { path, text, offset }
  }
  const anchor = endpoint(selection.anchorNode, selection.anchorOffset), focus = endpoint(selection.focusNode, selection.focusOffset)
  return anchor && focus ? { anchor, focus, text: selection.toString() } : null
}
export function restoreMirror2Range(doc: Document, saved: Mirror2Range | null, indexed: (node: Node) => boolean, protectedNode: (node: Node) => boolean): boolean {
  if (!saved) return false
  const resolve = (endpoint: Endpoint): Node | null => {
    let node: Node = doc.documentElement
    for (const index of endpoint.path) { const child = [...node.childNodes].filter(indexed)[index]; if (!child) return null; node = child }
    return node.nodeType === 3 && node.textContent === endpoint.text && endpoint.offset <= endpoint.text.length ? node : null
  }
  const anchor = resolve(saved.anchor), focus = resolve(saved.focus)
  if (!anchor || !focus) return false
  const range = doc.createRange(), backwards = anchor === focus ? saved.anchor.offset > saved.focus.offset : !!(anchor.compareDocumentPosition(focus) & Node.DOCUMENT_POSITION_PRECEDING)
  range.setStart(backwards ? focus : anchor, backwards ? saved.focus.offset : saved.anchor.offset)
  range.setEnd(backwards ? anchor : focus, backwards ? saved.anchor.offset : saved.focus.offset)
  if (range.toString() !== saved.text) return false
  const walk = doc.createTreeWalker(range.commonAncestorContainer, NodeFilter.SHOW_ELEMENT)
  let node: Node | null = walk.currentNode
  do { if (protectedNode(node) && range.intersectsNode(node)) return false } while ((node = walk.nextNode()))
  doc.getSelection()?.setBaseAndExtent(anchor, saved.anchor.offset, focus, saved.focus.offset)
  return true
}
