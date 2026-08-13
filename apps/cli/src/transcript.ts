export {
  buildApplyPatchNewPreview,
  formatToolDisplay,
  formatToolTranscriptUpdate,
  guessPathFenceLanguage,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  readApplyPatchFiles,
  splitInlineCodeSpans,
  type ApplyPatchFile,
  type InlineCodeSpan,
  type ToolDisplay,
  type ToolDisplayBlock,
  type ToolDisplayPatchFile,
  type ToolDisplayPatchLine,
  type ToolTranscriptUpdate,
} from "@chariox/tool-display"

export {
  shouldRenderProviderStatus,
} from "@chariox/kernel-client/provider-status"

export {
  normalizeMarkdownFenceInfoStrings,
  shouldRenderTranscriptAsMarkdown,
} from "./transcript-markdown.js"
