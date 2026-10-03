// Safe previews. Markdown is escaped first and only this renderer emits tags;
// HTML is parsed into an inert document and rebuilt from an allowlist with no
// attributes except in-page links, so no script, handler, style, frame or
// remote resource from a document can run in the App view.

const escape = (text) => text.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c])

function inline(text) {
  return escape(text)
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/\*([^*]+)\*/g, "<em>$1</em>")
}

export function markdownToHtml(markdown) {
  const out = []
  let list = null
  let code = null
  const closeList = () => { if (list) { out.push(`</${list}>`); list = null } }
  for (const line of markdown.replace(/\r\n?/g, "\n").split("\n")) {
    if (code !== null) {
      if (line.startsWith("```")) { out.push(`<pre><code>${escape(code.join("\n"))}</code></pre>`); code = null }
      else code.push(line)
      continue
    }
    if (line.startsWith("```")) { closeList(); code = []; continue }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line)
    const bullet = /^\s*[-*]\s+(.*)$/.exec(line)
    const numbered = /^\s*\d+\.\s+(.*)$/.exec(line)
    if (heading) { closeList(); out.push(`<h${heading[1].length}>${inline(heading[2])}</h${heading[1].length}>`) }
    else if (bullet || numbered) {
      const kind = bullet ? "ul" : "ol"
      if (list !== kind) { closeList(); out.push(`<${kind}>`); list = kind }
      out.push(`<li>${inline((bullet ?? numbered)[1])}</li>`)
    } else if (/^\s*>\s?/.test(line)) { closeList(); out.push(`<blockquote>${inline(line.replace(/^\s*>\s?/, ""))}</blockquote>`) }
    else if (line.trim() === "") closeList()
    else { closeList(); out.push(`<p>${inline(line)}</p>`) }
  }
  if (code !== null) out.push(`<pre><code>${escape(code.join("\n"))}</code></pre>`)
  closeList()
  return out.join("\n")
}

const ALLOWED = new Set(["P", "BR", "HR", "H1", "H2", "H3", "H4", "H5", "H6", "UL", "OL", "LI", "STRONG", "B", "EM", "I",
  "U", "CODE", "PRE", "BLOCKQUOTE", "TABLE", "THEAD", "TBODY", "TR", "TH", "TD", "SPAN", "DIV", "A", "SECTION", "ARTICLE"])
const DROP_WITH_CONTENT = new Set(["SCRIPT", "STYLE", "TEMPLATE", "IFRAME", "OBJECT", "EMBED", "NOSCRIPT", "SVG", "MATH"])

/** Rebuilds `html` as safe nodes owned by `target` (a document in the page). */
export function sanitizeHtmlInto(html, target, parse = (text) => new DOMParser().parseFromString(text, "text/html")) {
  const source = parse(html)
  const copy = (node, into) => {
    for (const child of node.childNodes) {
      if (child.nodeType === 3) { into.append(target.createTextNode(child.textContent)); continue }
      // Foreign (SVG, MathML) element names keep their case, so compare upper-cased.
      const name = child.nodeType === 1 ? child.nodeName.toUpperCase() : ""
      if (!name || DROP_WITH_CONTENT.has(name)) continue
      if (!ALLOWED.has(name)) { copy(child, into); continue }
      const element = target.createElement(name.toLowerCase())
      const href = name === "A" ? child.getAttribute("href") : null
      if (href && /^#[\w-]*$/.test(href)) element.setAttribute("href", href)
      copy(child, element)
      into.append(element)
    }
  }
  const fragment = target.createDocumentFragment()
  copy(source.body, fragment)
  return fragment
}
