import { dirname, extname } from "node:path";

// These exact consumers sort matching basenames and join with no separator.
// A changed/unknown consumer leaves an explicit gap rather than assuming it.
export const FRAGMENT_ASSEMBLIES = Object.freeze([
  { directory: "packages/tool-display/src/index-fragments", suffix: ".tsfrag",
    dependencies: [["packages/tool-display/scripts/build-fragments.mjs", "ae72d6fae1ef1e957a39c01417eeb5d7efefcc6b"]] },
  ...[
    ["browser-relay-kernel-drill", "ad640a15fb5bb5019f5942d378e49403761104e8"],
    ["staging-workflow-round-robin-live-drill", "9d0fadde4b4a0ff2a023705fad2a6771b8774dd0"],
    ["terminal-badge-drill", "0c8911093761f33cd932ac203fea0daf296a3cb5"],
  ].map(([name, blob]) => ({
    directory: "scripts/" + name + "-fragments", suffix: ".mjsfrag",
    dependencies: [["scripts/lib/run-fragmented-script.mjs", "0e84d48c7be6e26343039fce0205e9653fc72311"],
      ["scripts/" + name + ".mjs", blob]],
  })),
]);

function lineStarts(text) {
  const starts = [0];
  for (let index = 0; index < text.length; index++) if (text[index] === "\n") starts.push(index + 1);
  return starts;
}

function physicalAnchor(file, offset) {
  const starts = lineStarts(file.text);
  let index = 0;
  while (index + 1 < starts.length && starts[index + 1] <= offset) index++;
  return { path: file.path, blob: file.blob, line: index + 1, column: offset - starts[index] + 1,
    sourceLine: file.text.split(/\r?\n/)[index].trim() };
}

export function fragmentMatchAnchor(file, lineIndex, column, length) {
  const offset = file.assembly.lineStarts[lineIndex] + column - 1;
  const segments = [];
  let anchor;
  for (const span of file.assembly.spans) {
    const start = Math.max(offset, span.start);
    const end = Math.min(offset + length, span.end);
    if (end <= start) continue;
    const physical = physicalAnchor(span.file, start - span.start);
    anchor ??= physical;
    const { sourceLine, ...location } = physical;
    segments.push({ ...location, length: end - start });
  }
  if (!anchor) throw new Error("assembled fragment match has no physical anchor");
  return { ...anchor, fragmentSource: {
    directory: file.assembly.directory, assemblyStatus: "verified_sort_join_empty",
    dependencies: file.assembly.dependencies, members: file.assembly.spans.map(({ file }) => ({ path: file.path, blob: file.blob })),
    assembledLine: lineIndex + 1, assembledColumn: column, matchSegments: segments,
  } };
}

export function fragmentSourceViews(files) {
  const byPath = new Map(files.map((file) => [file.path, file]));
  const groups = new Map();
  const normal = [];
  for (const file of files) {
    const suffix = extname(file.path);
    if (![".tsfrag", ".mjsfrag"].includes(suffix)) { normal.push(file); continue; }
    const key = dirname(file.path) + ":" + suffix;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(file);
  }
  const gaps = [];
  const assemblies = [];
  for (const members of groups.values()) {
    members.sort((left, right) => left.path < right.path ? -1 : left.path > right.path ? 1 : 0);
    const directory = dirname(members[0].path);
    const suffix = extname(members[0].path);
    const rule = FRAGMENT_ASSEMBLIES.find((entry) => entry.directory === directory && entry.suffix === suffix);
    if (!rule || rule.dependencies.some(([path, blob]) => byPath.get(path)?.blob !== blob)) {
      gaps.push({ kind: "fragment_assembly_unresolved", directory, suffix,
        reason: rule ? "assembly consumer is missing or changed" : "assembly order/consumer is not inspected",
        members: members.map(({ path, blob }) => ({ path, blob })) });
      normal.push(...members.map((file) => ({ ...file, unverifiedFragment: true })));
      continue;
    }
    let position = 0;
    const spans = members.map((file) => {
      const span = { file, start: position, end: position + file.text.length };
      position = span.end;
      return span;
    });
    const text = members.map((file) => file.text).join("");
    const assembly = { directory, suffix, dependencies: rule.dependencies,
      members: members.map(({ path, blob }) => ({ path, blob })) };
    assemblies.push({ ...assembly, status: "verified_sort_join_empty" });
    normal.push({ ...members[0], text, assembly: { ...assembly, spans, lineStarts: lineStarts(text) } });
  }
  return { files: normal, gaps, assemblies };
}
