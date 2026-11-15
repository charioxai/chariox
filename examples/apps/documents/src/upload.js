// The files the person chose in the browser's own chooser, read as the text the App imports.
// Limits are the App's (and the kernel file prompt's): eight files of at most 512 KiB each.
export const MAX_FILES = 8
export const MAX_BYTES = 512 * 1024

/** { files: [{ name, content }], skipped: [{ name, reason }] }; a file that is too big or not UTF-8 text is skipped with its reason. */
export async function readUploads(chosen) {
  const files = []
  const skipped = []
  for (const file of Array.from(chosen).slice(0, MAX_FILES)) {
    if (file.size > MAX_BYTES) {
      skipped.push({ name: file.name, reason: `${file.name} is larger than 512 KB` })
      continue
    }
    try {
      files.push({ name: file.name, content: new TextDecoder("utf-8", { fatal: true }).decode(await file.arrayBuffer()) })
    } catch {
      skipped.push({ name: file.name, reason: `${file.name} is not UTF-8 text` })
    }
  }
  for (const file of Array.from(chosen).slice(MAX_FILES)) skipped.push({ name: file.name, reason: `Only ${MAX_FILES} files are imported at a time` })
  return { files, skipped }
}
