// MP-08/MP-10/MP-11: preserve font-face objects when unrelated image URLs hydrate.
// CSSOM edits keep the source style node, cascade order and DOM selectors intact.
type MutableRule = CSSRule & { style?: CSSStyleDeclaration; selectorText?: string }
export class Mirror2FontStyle {
  private raw = ''
  private rendered = ''
  private dirty = true
  private sheet: CSSStyleSheet | null = null
  constructor(readonly node: Element) {}
  set(raw: string): void { if (raw !== this.raw) { this.raw = raw; this.dirty = true } }
  private current(): CSSStyleSheet | null { return (this.node as HTMLStyleElement).sheet ?? null }
  paint(resolve: (text: string) => string): void {
    const text = resolve(this.raw), sheet = this.current()
    if (!this.dirty && text === this.rendered && sheet === this.sheet) return
    const view = this.node.ownerDocument.defaultView as (Window & typeof globalThis) | null
    let edited = false
    if (!this.dirty && sheet && sheet === this.sheet && /@font-face\b/i.test(this.raw) && view?.CSSStyleSheet) {
      try {
        const parsed = new view.CSSStyleSheet(); parsed.replaceSync(text)
        const next = [...parsed.cssRules]
        if (next.length === sheet.cssRules.length) {
          for (let i = 0; i < next.length; i++) {
            const before = sheet.cssRules[i] as MutableRule, after = next[i] as MutableRule
            if (before.cssText === after.cssText) continue
            if (before.type === after.type && before.style && after.style && before.selectorText === after.selectorText) before.style.cssText = after.style.cssText
            else { sheet.deleteRule(i); sheet.insertRule(after.cssText, i) }
          }
          edited = true
        }
      } catch { /* use the original complete-sheet path if CSSOM editing is unavailable */ }
    }
    if (!edited) this.node.textContent = text
    this.rendered = text; this.sheet = this.current(); this.dirty = false
  }
  attach(): void {
    const sheet = this.current()
    if (sheet === this.sheet) return
    // Initial insertion parses the text set above. A moved/replaced sheet must
    // recover hydrated URLs that were written through CSSOM, not textContent.
    if (this.sheet && this.node.textContent !== this.rendered) this.node.textContent = this.rendered
    this.sheet = this.current()
  }
  remove(): void { this.sheet = null }
}

// MP-08/MP-10/MP-11: failed/unused font declarations do not describe the
// fonts used by visible text, controls or generated content. Let Chromium
// resolve the actual family, weight and unicode range for that rendered text.
export function mirror2RenderedFontReady(element: Element, style: CSSStyleDeclaration): boolean {
  const doc = element.ownerDocument
  const ready = (font: CSSStyleDeclaration, text: string): boolean => {
    if (!text.trim() || font.visibility !== 'visible' || Number(font.opacity) === 0) return true
    try { return !!font.font && doc.fonts.check(font.font, text) } catch { return false }
  }
  let text = [...element.childNodes].filter(node => node.nodeType === 3).map(node => node.textContent ?? '').join('')
  if (['input', 'textarea'].includes(element.localName)) {
    const control = element as HTMLInputElement | HTMLTextAreaElement
    text += control.value || control.placeholder
  }
  if (!ready(style, text)) return false
  for (const pseudo of ['::before', '::after']) {
    const generated = doc.defaultView?.getComputedStyle(element, pseudo), content = generated?.content
    if (generated && content && !['none', 'normal', '""', "''"].includes(content) && !content.startsWith('url(') && !ready(generated, content)) return false
  }
  return true
}

// MP-08/MP-10/MP-11: canonical counterpart of the isolated observer descriptor.
export function mirror2FontDescriptor(font: FontFace): string {
  return JSON.stringify([font.family.replace(/^(['"])(.*)\1$/, '$2').trim().toLowerCase(), font.style.trim().replace(/\s+/g, ' '), font.weight === 'normal' ? '400' : font.weight.trim().replace(/\s+/g, ' '), font.stretch === 'normal' ? '100%' : font.stretch.trim(), font.unicodeRange.replace(/\s+/g, '').toUpperCase()])
}
