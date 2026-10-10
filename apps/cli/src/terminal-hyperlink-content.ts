import { CodeRenderable, StyledText, TextRenderable, type CliRenderer, type Renderable, type TextChunk, type TextBuffer, type SyntaxStyle } from "@opentui/core"
import { TUI_MOUSE_ENABLED } from "./terminal-mouse-policy.js"

const codeChunks = new WeakMap<CodeRenderable, { content: string; chunks: TextChunk[] }>()

export function rememberTerminalLinkChunks(node: CodeRenderable, chunks: TextChunk[], content: string): TextChunk[] {
  if (chunks.some(chunk => chunk.link)) codeChunks.set(node, { content, chunks })
  else codeChunks.delete(node)
  return chunks
}

/** MP-08/MP-10/MP-11: OpenTUI 0.1.87 LinkTracker.clear releases a pool
 * reference per cell although it retained only one per buffer. Refresh linked
 * text before painting, so repaint never uses an expired native URL ID. Keep
 * the original chunks (including styles/labels) and no persistent URL store. */
export function installTerminalHyperlinkContent(renderer: CliRenderer): () => void {
  if (TUI_MOUSE_ENABLED || process.env.CHARIOX_TUI_HYPERLINKS === "off") return () => {}
  const refresh = async () => {
    const visit = (node: Renderable) => {
      if (!node.visible) return
      if (node.height > 0 && (node.y >= renderer.height || node.y + node.height <= 0)) return
      if (node instanceof TextRenderable || node instanceof CodeRenderable) {
        const cached = node instanceof CodeRenderable ? codeChunks.get(node) : undefined
        const chunks = node instanceof TextRenderable ? node.textNode.toChunks()
          : cached?.content === node.content ? cached.chunks : undefined
        if (chunks?.some(chunk => chunk.link)) {
          // Pinned adapter for protected native buffers; content is unchanged.
          const native = node as unknown as { textBuffer: TextBuffer; _textBufferSyntaxStyle: SyntaxStyle }
          native.textBuffer.setStyledText(new StyledText(chunks))
          native._textBufferSyntaxStyle.clearCache()
        }
      }
      for (const child of node.getChildren()) visit(child)
    }
    visit(renderer.root)
  }
  renderer.setFrameCallback(refresh)
  return () => renderer.removeFrameCallback(refresh)
}
