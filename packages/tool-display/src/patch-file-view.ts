import { parseUnifiedDiff, type DiffFileView, type DiffRow } from "./diff-view.js"
import type { ApplyPatchFile } from "./types.js"

// MP-08 / MP-10: the diff view of one tool-patch file, shared by the web and terminal clients. A file that has no diff
// (a binary write, a delete, a move) shows its title or, when the title only repeats the verb, its note.
export function patchFileView(file: Pick<ApplyPatchFile, "kind" | "filePath" | "diff"> & Partial<Pick<ApplyPatchFile, "title" | "note">>): DiffFileView {
  const parsed = file.diff === null ? [] : parseUnifiedDiff(file.diff)
  const rows: DiffRow[] = parsed.flatMap((part) => part.rows)
  const commonTitles = ["Patched", "Created", "Deleted", "Edited", "Wrote"].map((verb) => `${verb} ${file.filePath}`)
  if (file.title && !commonTitles.includes(file.title)) {
    rows.unshift({ kind: "meta", text: file.title })
  } else if (file.note) {
    rows.push({ kind: "meta", text: file.note })
  }
  return {
    path: file.filePath,
    previousPath: parsed[0]?.previousPath ?? null,
    change: file.kind,
    additions: parsed.reduce((sum, part) => sum + part.additions, 0),
    deletions: parsed.reduce((sum, part) => sum + part.deletions, 0),
    rows,
  }
}
