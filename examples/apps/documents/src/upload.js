// The files the person chose in the browser's own chooser, read as the text the App imports.
// Limits are the App's (and the kernel file prompt's): eight files of at most 512 KiB each.
export const MAX_FILES = 8
export const MAX_BYTES = 512 * 1024
// The App refuses a call whose serialized input is over 512 KiB; stay under it for the envelope.
export const MAX_CALL_BYTES = 480 * 1024

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

const callBytes = (files) => new TextEncoder().encode(JSON.stringify({ files })).length

/** { batches: [[file]], skipped } : the files in the fewest requests that each fit one App call; a file that cannot fit alone is skipped. */
export function batches(files) {
  const result = []
  const skipped = []
  let open = []
  for (const file of files) {
    if (callBytes([file]) > MAX_CALL_BYTES) {
      skipped.push({ name: file.name, reason: `${file.name} is too big to import here` })
      continue
    }
    if (open.length && callBytes([...open, file]) > MAX_CALL_BYTES) {
      result.push(open)
      open = []
    }
    open.push(file)
  }
  if (open.length) result.push(open)
  return { batches: result, skipped }
}
