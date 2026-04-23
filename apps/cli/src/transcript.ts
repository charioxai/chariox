export {
  buildApplyPatchNewPreview,
  formatToolDisplay,
  formatToolTranscriptUpdate,
  guessPathFenceLanguage,
  mergeToolTranscriptUpdate,
  parseToolTranscriptUpdate,
  readApplyPatchFiles,
  shouldRenderProviderStatus,
  shouldSkipConsecutiveTranscriptEntry,
  splitInlineCodeSpans,
  type ApplyPatchFile,
  type InlineCodeSpan,
  type ToolDisplay,
  type ToolDisplayBlock,
  type ToolDisplayPatchFile,
  type ToolDisplayPatchLine,
  type ToolTranscriptUpdate,
} from "@arroba/tool-display"

export {
  normalizeMarkdownFenceInfoStrings,
  shouldRenderTranscriptAsMarkdown,
} from "./transcript-markdown.js"
