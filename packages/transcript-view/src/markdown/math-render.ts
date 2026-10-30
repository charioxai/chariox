// MP-08 / MP-10 (PANE-09): lazy KaTeX rendering for Markdown math (see
// lazy-render-cache.ts). The raw source, with its delimiters, is the fallback
// while KaTeX loads and whenever it cannot parse the expression.
import { createLazyRenderer } from "./lazy-render-cache.js"

declare const __CHARIOX_MATH_RENDERER_URL__: string | undefined
declare const __CHARIOX_MATH_STYLESHEET_URL__: string | undefined

type MathInput = { readonly tex: string; readonly display: boolean }

// Bubbles from a math element whose KaTeX markup replaced its source, so the enclosing pane can keep following the tail.
export const mathRenderedEvent = "chariox-math-rendered"

let loadAttempt = 0

const math = createLazyRenderer<MathInput>({
  attribute: "data-math-key",
  available: () => definedUrl(typeof __CHARIOX_MATH_RENDERER_URL__ === "string" ? __CHARIOX_MATH_RENDERER_URL__ : undefined) !== null,
  maxInputChars: 4_000,
  load: async () => {
    const url = definedUrl(typeof __CHARIOX_MATH_RENDERER_URL__ === "string" ? __CHARIOX_MATH_RENDERER_URL__ : undefined)
    if (!url) return null
    // Math is committed only with its styles: a failed stylesheet rejects this load, so the
    // bounded retry path (and finally the raw source) applies instead of unstyled KaTeX HTML.
    await ensureMathStylesheet()
    const module = await import(/* @vite-ignore */ (++loadAttempt === 1 ? url : `${url}?retry=${loadAttempt}`)) as { renderMath?: (tex: string, display: boolean) => string }
    const renderMath = module.renderMath
    return renderMath ? async (input) => {
      try { return renderMath(input.tex, input.display) } catch { return null }
    } : null
  },
  apply: applyMath,
})

export function mathKey(tex: string, display: boolean): string {
  return math.key({ tex, display })
}

export function cachedMath(key: string): string | null | undefined {
  return math.cached(key)
}

export function requestMath(key: string, tex: string, display: boolean): void {
  math.request(key, { tex, display })
}

export const flushMath = math.flush

export function setMathRendererForTests(render: ((tex: string, display: boolean) => Promise<string | null>) | null): void {
  math.setRenderForTests(render ? (input) => render(input.tex, input.display) : null)
}

function applyMath(element: HTMLElement, html: string | null): void {
  if (html === null) {
    element.dataset.mathState = "error"
    element.title = "math could not be rendered; showing the source"
    return
  }
  element.innerHTML = html
  element.dataset.mathState = "done"
  const EventType = element.ownerDocument.defaultView?.Event
  if (EventType) element.dispatchEvent(new EventType(mathRenderedEvent, { bubbles: true }))
}

function ensureMathStylesheet(): Promise<void> {
  const href = definedUrl(typeof __CHARIOX_MATH_STYLESHEET_URL__ === "string" ? __CHARIOX_MATH_STYLESHEET_URL__ : undefined)
  if (!href || typeof document === "undefined") return Promise.resolve()
  // The React shell and terminal runtime bundle this module separately: share the document's link.
  let link = document.querySelector<HTMLLinkElement>("link[data-chariox-katex]")
  if (link?.sheet) return Promise.resolve()
  const existing = link
  if (!link) {
    link = document.createElement("link")
    link.rel = "stylesheet"
    link.href = href
    link.dataset.charioxKatex = "true"
  }
  const pending = link
  return new Promise<void>((resolve, reject) => {
    pending.addEventListener("load", () => resolve(), { once: true })
    // A failed link is dropped so the next attempt requests the stylesheet again.
    pending.addEventListener("error", () => {
      pending.remove()
      reject(new Error("KaTeX stylesheet failed to load"))
    }, { once: true })
    if (!existing) document.head.append(pending)
  })
}

function definedUrl(value: string | undefined): string | null {
  return value ? value : null
}
