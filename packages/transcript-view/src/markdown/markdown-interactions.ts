import { handleMarkdownImageClick, handleMarkdownImageLoadClick } from "./markdown-images.js"

// MP-08 / MP-10 (PANE-09): delegated handlers for rendered Markdown. One
// listener per document serves every pane, history page and bundle (the
// terminal runtime and the React shell may both render transcripts).

const installedFlag = "charioxMarkdownInteractions"
const copiedResetMs = 1200

export function ensureMarkdownInteractions(target: Document | undefined = globalThis.document): void {
  if (!target?.documentElement || target.documentElement.dataset[installedFlag] === "true") return
  target.documentElement.dataset[installedFlag] = "true"
  target.addEventListener("click", (event) => {
    const Element = target.defaultView?.Element
    if (!Element || !(event.target instanceof Element)) return
    const imageLoad = event.target.closest<HTMLElement>("[data-markdown-image-load]")
    if (imageLoad && handleMarkdownImageLoadClick(imageLoad)) {
      event.preventDefault()
      event.stopPropagation()
      return
    }
    const image = event.target.closest<HTMLImageElement>("img.freeform-md-img")
    if (image && handleMarkdownImageClick(image)) {
      event.stopPropagation()
      return
    }
    const button = event.target.closest<HTMLElement>("[data-markdown-copy]")
    if (!button) return
    event.preventDefault()
    event.stopPropagation()
    const text = button.dataset.markdownCopy === "table"
      ? tableMarkdown(button.closest(".freeform-md-table")?.querySelector("table") ?? null)
      : button.closest("figure")?.querySelector("code[data-copy-source]")?.textContent
        ?? button.closest("figure")?.querySelector("code")?.textContent ?? ""
    void copyText(target, text).then((copied) => {
      button.textContent = copied ? "copied" : "copy failed"
      target.defaultView?.setTimeout(() => {
        button.textContent = "copy"
      }, copiedResetMs)
    })
  }, { capture: true })
}

export function tableMarkdown(table: HTMLTableElement | null): string {
  if (!table) return ""
  const rows = [...table.rows].map((row) => [...row.cells].map((cell) => (cell.textContent ?? "").trim().replace(/\|/g, "\\|").replace(/\s*\n\s*/g, " ")))
  if (rows.length === 0) return ""
  const width = Math.max(...rows.map((row) => row.length))
  const line = (cells: string[]) => `| ${Array.from({ length: width }, (_, index) => cells[index] ?? "").join(" | ")} |`
  const [header, ...body] = rows
  return [line(header!), line(Array.from({ length: width }, () => "---")), ...body.map(line)].join("\n")
}

async function copyText(target: Document, text: string): Promise<boolean> {
  try {
    await target.defaultView?.navigator.clipboard.writeText(text)
    return true
  } catch {
    return false
  }
}
