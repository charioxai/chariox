// MP-08 / MP-10 / MP-11 (PANE-09): lazily loaded KaTeX chunk (bundled, no CDN).
// `trust: false` disables \href, \url, \includegraphics and \html* commands;
// `throwOnError` lets the caller keep the raw source on any parse error.
import katex from "katex"

export function renderMath(tex: string, display: boolean): string {
  return katex.renderToString(tex, {
    displayMode: display,
    throwOnError: true,
    trust: false,
    strict: "ignore",
    output: "htmlAndMathml",
    maxSize: 20,
    maxExpand: 500,
  })
}
