// Decode unified diffs without treating removed source as installed behavior.
// Every view preserves the physical patch line and its embedded source identity.
export function patchSourceViews(file, classifyPath) {
  const raw = file.text.split(/\r?\n/);
  const views = [];
  let view = null;
  let hunk = null;
  const finishHunk = () => {
    if (hunk && (hunk.oldRemaining || hunk.newRemaining)) {
      throw new Error(`incomplete patch hunk: ${file.path}`);
    }
    hunk = null;
  };
  const safePath = (value, prefix = null) => {
    if (value === "/dev/null") return null;
    if (prefix && value.startsWith(prefix)) value = value.slice(prefix.length);
    if (!value || /[\\\s\x00-\x1f\x7f:]/.test(value)
      || value.split("/").some((part) => !part || part === "." || part === "..")) {
      throw new Error(`unsafe patch source path: ${file.path}`);
    }
    return value;
  };
  const sourceFormat = (path) => {
    if (path === null) return null;
    const format = classifyPath(path);
    if (format === "patch") throw new Error(`nested patch source is unsupported: ${file.path}`);
    return format;
  };
  const beginSection = (oldPath, newPath, index, gitHeader = false) => {
    finishHunk();
    if (oldPath === null && newPath === null) throw new Error(`empty patch source paths: ${file.path}`);
    view = { path: newPath ?? oldPath, oldPath, newPath,
      format: sourceFormat(newPath ?? oldPath), oldFormat: sourceFormat(oldPath),
      lineOffset: index, lines: [], patchLines: [], hunkStarts: [], gitHeader, headers: false };
    views.push(view);
  };
  for (let index = 0; index < raw.length; index += 1) {
    const line = raw[index];
    const inHunk = hunk && (hunk.oldRemaining || hunk.newRemaining);
    if (!inHunk && line.startsWith("diff --git ")) {
      const match = /^diff --git (a\/\S+) (b\/\S+)$/.exec(line);
      if (!match) throw new Error(`unsupported patch source header: ${file.path}:${index + 1}`);
      beginSection(safePath(match[1], "a/"), safePath(match[2], "b/"), index, true);
    } else if (!inHunk && line.startsWith("--- ")) {
      if (!raw[index + 1]?.startsWith("+++ ")) {
        throw new Error(`missing new patch source header: ${file.path}:${index + 1}`);
      }
      // MP-11: ordinary unified headers may have tab-delimited timestamps.
      const oldPath = safePath(line.slice(4).split("\t")[0], "a/");
      const newPath = safePath(raw[index + 1].slice(4).split("\t")[0], "b/");
      if (view?.gitHeader && !view.headers && view.hunkStarts.length === 0) {
        if ((oldPath !== null && oldPath !== view.oldPath)
          || (newPath !== null && newPath !== view.newPath)) {
          throw new Error(`patch source header mismatch: ${file.path}:${index + 1}`);
        }
        // /dev/null on either side marks an actual addition/deletion.
        view.oldPath = oldPath;
        view.newPath = newPath;
        view.oldFormat = sourceFormat(oldPath);
        view.format = sourceFormat(newPath ?? oldPath);
      } else beginSection(oldPath, newPath, index);
      view.headers = true;
      view.lines[index - view.lineOffset] = "";
      view.lines[index + 1 - view.lineOffset] = "";
      index += 1;
      continue;
    }
    if (!view) {
      if (line.trim()) throw new Error(`missing patch source header: ${file.path}`);
      continue;
    }
    const local = index - view.lineOffset;
    view.lines[local] = "";
    if (line.startsWith("@@")) {
      finishHunk();
      view.hunkStarts.push(local);
      const match = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(?:.*)$/.exec(line);
      if (!match) throw new Error(`unsupported patch hunk: ${file.path}:${index + 1}`);
      const counts = match.slice(1).map((value) => Number(value ?? 1));
      if (counts.some((value) => !Number.isSafeInteger(value))) throw new Error(`invalid patch line count: ${file.path}`);
      hunk = { oldLine: counts[0], oldRemaining: counts[1], newLine: counts[2], newRemaining: counts[3] };
      if ((view.oldPath === null && hunk.oldRemaining !== 0)
        || (view.newPath === null && hunk.newRemaining !== 0)
        || (!hunk.oldRemaining && !hunk.newRemaining)
        || (hunk.oldRemaining && !hunk.oldLine) || (hunk.newRemaining && !hunk.newLine)) {
        throw new Error(`patch source line count mismatch: ${file.path}`);
      }
      if (hunk.oldLine < (view.oldEnd ?? 0) || hunk.newLine < (view.newEnd ?? 0)) {
        throw new Error(`overlapping patch hunks: ${file.path}`);
      }
      view.oldEnd = hunk.oldLine + hunk.oldRemaining;
      view.newEnd = hunk.newLine + hunk.newRemaining;
      continue;
    }
    if (line === "\\ No newline at end of file") {
      if (!view.patchLines[local - 1]) throw new Error(`orphan patch newline marker: ${file.path}`);
      continue;
    }
    if (!hunk || (!hunk.oldRemaining && !hunk.newRemaining)) {
      if (hunk) finishHunk();
      if (!line || /^(?:diff --git |index |new file mode |deleted file mode |old mode |new mode |similarity index )/.test(line)) continue;
      if (/^rename (?:from|to) /.test(line)) {
        const role = line.startsWith("rename from ") ? "oldPath" : "newPath";
        if (safePath(line.replace(/^rename (?:from|to) /, "")) === view[role]) continue;
      }
      throw new Error(`unsupported patch metadata: ${file.path}:${index + 1}`);
    }
    const change = ({ "+": "added", "-": "removed", " ": "context" })[line[0]];
    if (!change) throw new Error(`invalid patch line: ${file.path}:${index + 1}`);
    const source = { path: change === "removed" ? view.oldPath : view.newPath, change,
      oldLine: change === "added" ? null : hunk.oldLine,
      newLine: change === "removed" ? null : hunk.newLine };
    if (change !== "added") { hunk.oldLine++; hunk.oldRemaining--; }
    if (change !== "removed") { hunk.newLine++; hunk.newRemaining--; }
    if (hunk.oldRemaining < 0 || hunk.newRemaining < 0) {
      throw new Error(`patch line count mismatch: ${file.path}:${index + 1}`);
    }
    view.lines[local] = line.slice(1);
    view.patchLines[local] = source;
  }
  finishHunk();
  if (!views.length) throw new Error(`patch has no source sections: ${file.path}`);
  if (views.some((section) => !section.hunkStarts.length)) throw new Error(`patch section has no source hunks: ${file.path}`);
  // Omitted source between hunks has unknown lexical state. Keep each hunk
  // separate so an opener cannot mask later executable source when its closing
  // delimiter was omitted from the diff. Unknown context stays conservative.
  const regions = views.filter((view) => view.format || view.oldFormat).flatMap((view) =>
    view.hunkStarts.map((start, index) => {
      const end = view.hunkStarts[index + 1] ?? view.lines.length;
      return { ...view, lineOffset: view.lineOffset + start,
        lines: view.lines.slice(start, end), patchLines: view.patchLines.slice(start, end) };
    }));
  return regions.flatMap((view) => {
    // Old and new sides have independent comment state. Removed comments must
    // never mask added source, and context candidates are emitted only once.
    const result = [];
    const active = view.newPath !== null && view.format;
    if (active) result.push({ ...view, path: view.newPath,
      text: view.lines.map((line, index) => view.patchLines[index]?.change === "removed" ? "" : line).join("\n") });
    if (view.oldFormat && (view.patchLines.some((line) => line?.change === "removed") || !active)) {
      result.push({ ...view, path: view.oldPath, format: view.oldFormat, onlyRemoved: Boolean(active), oldOnly: !active,
        patchLines: view.patchLines.map((line) => line ? { ...line, path: view.oldPath } : line),
        text: view.lines.map((line, index) => view.patchLines[index]?.change === "added" ? "" : line).join("\n") });
    }
    return result;
  });
}
