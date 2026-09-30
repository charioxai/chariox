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
  for (let index = 0; index < raw.length; index += 1) {
    const line = raw[index];
    if (line.startsWith("diff --git ")) {
      finishHunk();
      const match = /^diff --git a\/(\S+) b\/(\S+)$/.exec(line);
      if (!match || match[1].split("/").includes("..") || match[2].split("/").includes("..")) {
        throw new Error(`unsupported patch source header: ${file.path}:${index + 1}`);
      }
      const path = match[2];
      const format = classifyPath(path);
      if (format === "patch") throw new Error(`nested patch source is unsupported: ${file.path}`);
      view = { path, format, lineOffset: index, lines: [], patchLines: [] };
      views.push(view);
    }
    if (!view) {
      if (line.trim()) throw new Error(`missing patch source header: ${file.path}`);
      continue;
    }
    const local = index - view.lineOffset;
    view.lines[local] = "";
    if (line.startsWith("@@")) {
      finishHunk();
      const match = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(?:.*)$/.exec(line);
      if (!match) throw new Error(`unsupported patch hunk: ${file.path}:${index + 1}`);
      hunk = {
        oldLine: Number(match[1]), oldRemaining: Number(match[2] ?? 1),
        newLine: Number(match[3]), newRemaining: Number(match[4] ?? 1),
      };
      continue;
    }
    if (!hunk || (!hunk.oldRemaining && !hunk.newRemaining)) {
      if (line.startsWith("\\ No newline")) continue;
      if (hunk) finishHunk();
      if (!line || /^(?:diff --git |index |--- |\+\+\+ |new file mode |deleted file mode |old mode |new mode |similarity index |rename from |rename to )/.test(line)) continue;
      throw new Error(`unsupported patch metadata: ${file.path}:${index + 1}`);
    }
    if (line.startsWith("\\ No newline")) continue;
    const change = ({ "+": "added", "-": "removed", " ": "context" })[line[0]];
    if (!change) throw new Error(`invalid patch line: ${file.path}:${index + 1}`);
    const source = { path: view.path, change,
      oldLine: change === "added" ? null : hunk.oldLine,
      newLine: change === "removed" ? null : hunk.newLine,
    };
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
  return views.filter((view) => view.format).flatMap((view) => {
    // Old and new sides have independent comment state. Removed comments must
    // never mask added source, and context candidates are emitted only once.
    const active = { ...view,
      text: view.lines.map((line, index) => view.patchLines[index]?.change === "removed" ? "" : line).join("\n"),
    };
    if (!view.patchLines.some((line) => line?.change === "removed")) return [active];
    return [active, { ...view, onlyRemoved: true,
      text: view.lines.map((line, index) => view.patchLines[index]?.change === "added" ? "" : line).join("\n"),
    }];
  });
}

