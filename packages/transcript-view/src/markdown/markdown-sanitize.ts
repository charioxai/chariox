import DOMPurify, { type DOMPurify as DOMPurifyInstance } from "dompurify"

// MP-08 / MP-10 / MP-11 (browser observation protection): provider text is
// untrusted. markdown-it runs with `html: false`, so raw provider HTML is
// escaped before it reaches this point; this allowlist is the second wall.
// No script/style/iframe/form elements, no event handlers, no inline styles
// except the highlighter's colour variables, and only vetted URL schemes.
const allowedTags = [
  "a", "blockquote", "br", "button", "code", "del", "details", "div", "em", "figcaption", "figure",
  "h1", "h2", "h3", "h4", "h5", "h6", "hr", "img", "li", "ol", "p", "pre", "s", "span",
  "strong", "summary", "table", "tbody", "td", "th", "thead", "tr", "ul",
]
const allowedAttributes = [
  "alt", "aria-checked", "aria-disabled", "aria-label", "class", "colspan", "hidden", "href", "rel",
  "role", "rowspan", "src", "start", "style", "tabindex", "target", "title", "type",
]
const highlightStylePattern = /^(?:--h[ld]:#[0-9a-f]{3,8}|font-style:italic|font-weight:bold|text-decoration:underline)(?:;(?:--h[ld]:#[0-9a-f]{3,8}|font-style:italic|font-weight:bold|text-decoration:underline))*$/
const allowedHrefPattern = /^(?:https?:|mailto:)/i
// Remote images never get a `src` from the renderer: they load only after a
// click (see markdown-interactions.ts), so a pane never leaks the viewer's IP.
const allowedImageSourcePattern = /^data:image\/(?:png|jpe?g|gif|webp);base64,/i
const allowedTextAlign = /^text-align:(?:left|right|center)$/

let purifier: DOMPurifyInstance | null | undefined

export function sanitizeMarkdownHtml(html: string): string {
  const instance = markdownPurifier()
  if (!instance) return html
  return instance.sanitize(html, {
    ALLOWED_TAGS: allowedTags,
    ALLOWED_ATTR: allowedAttributes,
    ALLOW_DATA_ATTR: true,
    ALLOW_ARIA_ATTR: false,
    ALLOW_UNKNOWN_PROTOCOLS: false,
  })
}

// Exposed for tests that bring their own window (jsdom).
export function setMarkdownSanitizerWindow(window: Window | null): void {
  purifier = window ? configure(DOMPurify(window as unknown as Window & typeof globalThis)) : undefined
}

function markdownPurifier(): DOMPurifyInstance | null {
  if (purifier !== undefined) return purifier
  purifier = typeof window === "undefined" ? null : configure(DOMPurify(window))
  return purifier
}

function configure(instance: DOMPurifyInstance): DOMPurifyInstance | null {
  if (!instance.isSupported) return null
  instance.addHook("uponSanitizeAttribute", (node, data) => {
    const name = data.attrName
    const value = data.attrValue.trim()
    if (name === "style") {
      const isHighlightSpan = node.nodeName === "SPAN" && highlightStylePattern.test(value)
      const isCellAlign = (node.nodeName === "TD" || node.nodeName === "TH") && allowedTextAlign.test(value)
      if (!isHighlightSpan && !isCellAlign) data.keepAttr = false
    } else if (name === "href") {
      if (!allowedHrefPattern.test(value)) data.keepAttr = false
    } else if (name === "src") {
      if (node.nodeName !== "IMG" || !allowedImageSourcePattern.test(value)) data.keepAttr = false
    } else if (name === "target") {
      if (value !== "_blank") data.keepAttr = false
    } else if (name === "type") {
      if (node.nodeName !== "BUTTON" || value !== "button") data.keepAttr = false
    }
  })
  instance.addHook("afterSanitizeAttributes", (node) => {
    if (node.nodeName === "A" && node.getAttribute("target") === "_blank") {
      node.setAttribute("rel", "noopener noreferrer nofollow")
    }
  })
  return instance
}
