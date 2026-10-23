// MP-08 / MP-10 (PANE-09): every agent-pane body (assistant text, reasoning,
// tool output, summaries) renders through the shared GFM content renderer.
export {
  renderMarkdownHtml as renderTranscriptHtml,
  renderMarkdownInlineHtml as renderTranscriptInlineHtml,
} from "./markdown/markdown-renderer.js"
