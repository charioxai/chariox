import assert from "node:assert/strict"
import test from "node:test"
import { renderMarkdownImage, setMarkdownWorkspaceImageLoader } from "./markdown-images.js"

const escapeHtml = (value: string) => value.replace(/[&<>"']/g, char => `&#${char.charCodeAt(0)};`)

// Review #1263: the trigger viewer sets no workspace loader, so a chart written as a workspace PNG read "loading…" for ever.
test("a workspace picture says loading only where a loader will fill it", () => {
  setMarkdownWorkspaceImageLoader(null)
  const unloaded = renderMarkdownImage("charts/cost.png", "Cost", escapeHtml)
  assert.doesNotMatch(unloaded, /loading/)
  assert.match(unloaded, /workspace · cost\.png/, "the reader still names the file")
  assert.match(unloaded, /data-md-image-state="pending"/, "a loader set later can still fill it")
  setMarkdownWorkspaceImageLoader(async () => ({ dataUrl: "data:image/png;base64,AA==" }) as never)
  assert.match(renderMarkdownImage("charts/cost.png", "Cost", escapeHtml), /loading…/)
  setMarkdownWorkspaceImageLoader(null)
})
