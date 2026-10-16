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
