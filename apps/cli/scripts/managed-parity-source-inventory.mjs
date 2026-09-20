#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, extname, isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const INVENTORY_SCHEMA = "chariox.managed-parity.source-inventory.v1";
export const REVIEWED_SOURCE_COMMIT = "391b38b2be15c4f49d8ea70cc14b031385cf8331";
export const REVIEWED_SOURCE_TREE = "199b565b83e9582acd38a38a76520e2ba5ac02b4";

export const MP_ROWS = Object.freeze([
  {
    id: "MP-01",
    name: "reviewed source and build identity",
    categories: ["source_identity", "release_activation"],
  },
  {
    id: "MP-02",
    name: "directory discovery and exact-path entry",
    categories: ["repository_home_state_path", "protected_path_filter"],
  },
  {
    id: "MP-03",
    name: "workspace, repository basename, and collision handling",
    categories: ["repository_home_state_path", "protected_path_filter"],
  },
  {
    id: "MP-04",
    name: "provider ancestry, environment, mounts, privilege, and network",
    categories: ["managed_env_selector", "bubblewrap", "managed_service_restriction"],
  },
  {
    id: "MP-05",
    name: "session, worktree, project, and client projection",
    categories: ["client_projection", "managed_only_branch"],
  },
  {
    id: "MP-06",
    name: "reconnect, history, queued prompts, and result identity",
    categories: ["client_projection", "managed_only_error_mapping"],
  },
  {
    id: "MP-07",
    name: "control-file protection, permissions, limits, and errors",
    categories: ["protected_path_filter", "managed_only_error_mapping"],
  },
  {
    id: "MP-08",
    name: "cleanup and revocation",
    categories: ["cleanup_selector"],
  },
  {
    id: "MP-09",
    name: "signed release activation",
    categories: ["release_activation"],
  },
  {
    id: "MP-10",
    name: "mandatory managed automatic shutdown",
    categories: ["automatic_shutdown_selector"],
  },
  {
    id: "MP-11",
    name: "proactive managed-only source inventory",
    categories: ["source_identity"],
  },
]);

export const REQUIRED_CATEGORIES = Object.freeze([
  "managed_only_branch",
  "managed_env_selector",
  "bubblewrap",
  "managed_service_restriction",
  "protected_path_filter",
  "managed_only_error_mapping",
  "repository_home_state_path",
  "client_projection",
  "cleanup_selector",
  "release_activation",
  "automatic_shutdown_selector",
]);

const SCANNABLE_EXTENSIONS = new Set([
  ".cjs",
  ".conf",
  ".js",
  ".json",
  ".mjs",
  ".rs",
  ".service",
  ".sh",
  ".toml",
  ".ts",
  ".tsx",
  ".yaml",
  ".yml",
]);

const IGNORED_DIRECTORY_NAMES = new Set([
  ".git",
  ".next",
  ".turbo",
  "build",
  "coverage",
  "dist",
  "node_modules",
  "target",
]);

const IGNORED_PATH_PARTS = [
  /(?:^|\/)docs(?:\/|$)/i,
  /(?:^|\/)(?:fixtures?|snapshots?)(?:\/|$)/i,
  /(?:^|\/)(?:test|tests|__tests__)(?:\/|$)/i,
  /(?:^|\/)apps\/cli\/scripts(?:\/|$)/i,
  /(?:^|\/)[^/]*test[^/]*\.[^/]+$/i,
];

const CATEGORY_SPECS = Object.freeze([
  {
    category: "managed_env_selector",
    mpIds: ["MP-04"],
    affectedBehavior: "managed environment selector or injected managed runtime marker",
    pattern: /\bCHARIOX_MANAGED[A-Z0-9_]*/g,
  },
  {
    category: "bubblewrap",
    mpIds: ["MP-04"],
    affectedBehavior: "Bubblewrap or equivalent provider ancestry/sandbox boundary",
    pattern: /(?:\/usr\/bin\/)?\bbwrap\b|\bbubblewrap\b/gi,
  },
  {
    category: "managed_service_restriction",
    mpIds: ["MP-04", "MP-07"],
    affectedBehavior: "managed service-unit or inherited system restriction",
    pattern: /\b(?:ProtectSystem|ProtectHome|PrivateTmp|NoNewPrivileges|Restrict[A-Za-z0-9_]*|ReadWritePaths|UMask)\b/g,
  },
  {
    category: "managed_only_branch",
    mpIds: ["MP-05"],
    affectedBehavior: "branch or mode switch that changes managed runtime behavior",
    pattern: /\b(?:is_managed|managed_mode|managed_only|managed_context|managedEnvironment|managedRuntime)\b|\b(?:if|else\s+if|match|switch|case)\b[^\n;{}]{0,120}\bmanaged\b/gi,
  },
  {
    category: "protected_path_filter",
    mpIds: ["MP-02", "MP-03", "MP-07"],
    affectedBehavior: "protected/control path or parent filtering",
    pattern: /\b(?:protected[_ -]?paths?|protected_parent|protected_path|control[_ -]?(?:root|file|path)|is_protected|force_excludes?|path_filter)\b/gi,
  },
  {
    category: "managed_only_error_mapping",
    mpIds: ["MP-06", "MP-07"],
    affectedBehavior: "managed-only error, failure, denial, or unsupported mapping",
    pattern: /\bmanaged\b[^\n;{}]{0,100}\b(?:error|failure|failed|denied|unsupported|invalid)\b|\b(?:error|failure|failed|denied|unsupported|invalid)\b[^\n;{}]{0,100}\bmanaged\b/gi,
  },
  {
    category: "repository_home_state_path",
    mpIds: ["MP-02", "MP-03"],
    affectedBehavior: "repository-root, home, provider-home, or durable state path",
    pattern: /\b(?:CHARIOX_HOME|CHARIOX_MANAGED_REPOSITORY_ROOT|managed[_-]?repository[_-]?root|repository[_-]?root|state[_-]?root|provider[_-]?home)\b|\/home\/chariox(?:\/[A-Za-z0-9._/-]+)?|\/var\/lib\/chariox(?:\/[A-Za-z0-9._/-]+)?|\.chariox\b/g,
  },
  {
    category: "client_projection",
    mpIds: ["MP-05", "MP-06"],
    affectedBehavior: "managed state projected to a client, response, UI, or result",
    pattern: /\bmanaged\b[^\n;{}]{0,100}\b(?:projection|client|response|payload|result|json|ui|web|history|prompt)\b|\b(?:projection|client|response|payload|result|json|ui|web|history|prompt)\b[^\n;{}]{0,100}\bmanaged\b/gi,
  },
  {
    category: "cleanup_selector",
    mpIds: ["MP-08"],
    affectedBehavior: "managed cleanup, revocation, expiry, reaping, or deletion path",
    pattern: /\bmanaged\b[^\n;{}]{0,100}\b(?:cleanup|revoke|revocation|expire|expiry|reap|delete|dispose)\b|\b(?:cleanup|revoke|revocation|expire|expiry|reap|delete|dispose)\b[^\n;{}]{0,100}\bmanaged\b/gi,
  },
  {
    category: "release_activation",
    mpIds: ["MP-01", "MP-09"],
    affectedBehavior: "signed managed release activation and artifact identity",
    pattern: /\b(?:verify_release|release[-_ ]manifest|release[-_ ]signature|release[-_]public[-_]key|signed release|release_digest|kernel artifact)\b/gi,
  },
  {
    category: "automatic_shutdown_selector",
    mpIds: ["MP-10"],
    affectedBehavior: "managed automatic shutdown policy or trigger selector",
    pattern: /\b(?:auto[_-]?stop|automatic[_-]?shutdown|idle[_-]?delay|minimum[_-]?runtime|keep[_-]?running|last[_-]?agent|agents[_-]?done)(?:[_-][A-Za-z0-9]+)*\b|\b(?:managed|idle|minimum|deployment|lifecycle)\b[^\n;{}]{0,100}\bshutdown\b/gi,
  },
]);

// These are the only reviewed managed-only exemptions. They are deliberately
// bound to the exact reviewed source identity, relative path, symbol, line,
// and trimmed source line. A changed line stops matching and becomes red.
export const DEFAULT_REVIEWED_PREDICATES = Object.freeze([
  {
    id: "MP-09-release-verify-release",
    sourceCommit: REVIEWED_SOURCE_COMMIT,
    sourceTree: REVIEWED_SOURCE_TREE,
    path: "apps/kernel/src/managed_bootstrap/release.rs",
    line: 39,
    symbol: "verify_release",
    sourceLine: "pub(super) fn verify_release(",
    category: "release_activation",
    topology: "direct_path1",
    applicableMpIds: ["MP-01", "MP-09"],
    disposition: "allowed_release_deployment",
  },
  {
    id: "MP-10-auto-stop-policy",
    sourceCommit: REVIEWED_SOURCE_COMMIT,
    sourceTree: REVIEWED_SOURCE_TREE,
    path: "apps/kernel/src/runtime/managed_environment_control/cloud_contract.rs",
    line: 110,
    symbol: "AutoStopPolicy",
    sourceLine: "minimum_runtime_seconds: u64,",
    category: "automatic_shutdown_selector",
    topology: "direct_path1",
    applicableMpIds: ["MP-10"],
    disposition: "required_automatic_shutdown",
  },
  {
    id: "MP-10-auto-stop-idle-delay",
    sourceCommit: REVIEWED_SOURCE_COMMIT,
    sourceTree: REVIEWED_SOURCE_TREE,
    path: "apps/kernel/src/runtime/managed_environment_control/cloud_contract.rs",
    line: 111,
    symbol: "AutoStopPolicy",
    sourceLine: "idle_delay_seconds: Option<u64>,",
    category: "automatic_shutdown_selector",
    topology: "direct_path1",
    applicableMpIds: ["MP-10"],
    disposition: "required_automatic_shutdown",
  },
]);

const ALLOWED_DISPOSITIONS = new Set([
  "allowed_release_deployment",
  "required_automatic_shutdown",
]);

const DISPOSITIONS = new Set([
  ...ALLOWED_DISPOSITIONS,
  "removal_required",
  "unreviewed",
]);

const DIRECT_PATH1_PREFIXES = [
  "apps/kernel/src/managed_bootstrap/",
  "apps/kernel/src/managed_context/",
  "apps/kernel/src/provider/",
  "apps/kernel/src/runtime/managed_environment_control",
  "apps/kernel/src/runtime/state/",
  "deploy/managed-kernel/",
];

const OWNED_DIRTY_PATHS = new Set([
  "apps/cli/package.json",
  "apps/cli/scripts/managed-parity-source-inventory.mjs",
  "apps/cli/scripts/managed-parity-source-inventory.test.mjs",
]);

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function stable(value) {
  if (Array.isArray(value)) return value.map(stable);
  if (!value || typeof value !== "object") return value;
  return Object.fromEntries(Object.keys(value).sort().map((key) => [key, stable(value[key])]));
}

export function stableJson(value) {
  return `${JSON.stringify(stable(value), null, 2)}\n`;
}

function redact(value) {
  return String(value)
    .replace(/sk-[A-Za-z0-9_-]+/g, "sk-<redacted>")
    .replace(/((?:token|secret|password|credential|api[_-]?key)\s*[=:]\s*["']?)[^\s,"']+/gi, "$1<redacted>")
    .replace(/[\u0000-\u001f\u007f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 160);
}

function defaultRunGit(args, cwd) {
  const result = spawnSync("git", args, { cwd, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`git ${args.join(" ")} failed with status ${result.status}`);
  }
  return result.stdout ?? "";
}

function parseLsTree(output) {
  return output.split("\0").filter(Boolean).map((record) => {
    const match = /^(\d+)\s+(blob|commit|tree)\s+([0-9a-f]{40})\t(.+)$/.exec(record);
    if (!match) throw new Error("invalid git ls-tree record");
    return { mode: match[1], type: match[2], blob: match[3], path: match[4] };
  }).filter((entry) => entry.type === "blob");
}

function parseStatus(output) {
  return output.split("\0").filter(Boolean).map((record) => {
    const path = record.length > 3 && record[2] === " " ? record.slice(3) : record.slice(3);
    return path.includes(" -> ") ? path.slice(path.indexOf(" -> ") + 4) : path;
  });
}

function isIgnoredPath(path) {
  return IGNORED_PATH_PARTS.some((pattern) => pattern.test(path));
}

function isScannable(path) {
  if (isIgnoredPath(path)) return false;
  const parts = path.split("/");
  if (parts.some((part) => IGNORED_DIRECTORY_NAMES.has(part))) return false;
  const extension = extname(path).toLowerCase();
  return SCANNABLE_EXTENSIONS.has(extension) || path.endsWith(".service.in");
}

function readText(fsApi, absolutePath) {
  const value = fsApi.readFileSync(absolutePath);
  if (value.includes(0)) return null;
  return value.toString("utf8");
}

function inferSymbol(lines, lineIndex) {
  for (let index = lineIndex; index >= Math.max(0, lineIndex - 12); index -= 1) {
    const line = lines[index];
    const match = /\b(?:fn|function|class|struct|enum|const|static|impl|mod)\s+([A-Za-z0-9_:-]+)/.exec(line);
    if (match) return match[1].replace(/:+$/, "");
  }
  return null;
}

function inferTopology(path, lines, lineIndex) {
  const context = lines.slice(Math.max(0, lineIndex - 3), lineIndex + 4).join(" ").toLowerCase();
  const lowerPath = path.toLowerCase();
  if (lowerPath.includes("slice-linux-docker") || lowerPath.includes("/slice/local_docker/") || /inner[ -]?docker|docker[ -]?slice|nested docker/.test(context)) {
    return "inner_docker_slice";
  }
  if (lowerPath.includes("legacy") || lowerPath.includes("shared-host") || lowerPath.includes("shared_host") || /legacy shared[ -]?host/.test(context)) {
    return "legacy_shared_host";
  }
  if (DIRECT_PATH1_PREFIXES.some((prefix) => path.startsWith(prefix)) || path.startsWith("apps/kernel/src/")) return "direct_path1";
  return "unknown";
}

function lineMatches(spec, line) {
  spec.pattern.lastIndex = 0;
  return [...line.matchAll(spec.pattern)].map((match) => ({ value: match[0], index: match.index ?? 0 }));
}

function predicateMatches(finding, predicate, source) {
  return predicate.sourceCommit === source.commit
    && predicate.sourceTree === source.tree
    && predicate.path === finding.path
    && predicate.line === finding.line
    && predicate.symbol === finding.symbol
    && predicate.category === finding.category
    && predicate.sourceLine === finding.sourceLine;
}

function baseDisposition(finding) {
  if (finding.topology !== "direct_path1") return "unreviewed";
  if (["managed_env_selector", "bubblewrap", "managed_service_restriction", "managed_only_branch", "protected_path_filter", "managed_only_error_mapping", "client_projection"].includes(finding.category)) {
    return "removal_required";
  }
  return "unreviewed";
}

function assertDispositionClaims(claims, predicates, source) {
  for (const claim of claims ?? []) {
    if (!DISPOSITIONS.has(claim.disposition)) throw new Error("unverified disposition claim: invalid disposition");
    const verified = predicates.some((predicate) => predicateMatches({
      path: claim.path,
      line: claim.line,
      symbol: claim.symbol,
      category: claim.category,
      sourceLine: claim.sourceLine,
    }, predicate, source) && predicate.disposition === claim.disposition);
    if (!verified) throw new Error(`unverified disposition claim for ${claim.path}:${claim.line}`);
  }
}

function buildRowCoverage(entries, sourceIdentity) {
  const observed = new Set(["source_identity"]);
  for (const entry of entries) observed.add(entry.category);
  return MP_ROWS.map((row) => ({
    id: row.id,
    name: row.name,
    categories: row.categories,
    status: row.categories.every((category) => observed.has(category)) ? "observed" : "missing",
  }));
}

function collectTrackedFiles({ sourceRoot, sourceRef, fsApi, runGit }) {
  const treeEntries = parseLsTree(runGit(["ls-tree", "-r", "--full-tree", "-z", sourceRef], sourceRoot));
  return treeEntries.filter((entry) => isScannable(entry.path)).map((entry) => ({
    ...entry,
    text: sourceRef === "HEAD"
      ? readText(fsApi, join(sourceRoot, entry.path))
      : runGit(["show", `${sourceRef}:${entry.path}`], sourceRoot),
  })).filter((entry) => entry.text !== null);
}

export function reviewedPredicatesFor({ sourceCommit = REVIEWED_SOURCE_COMMIT, sourceTree = REVIEWED_SOURCE_TREE } = {}) {
  return DEFAULT_REVIEWED_PREDICATES.map((predicate) => ({ ...predicate, sourceCommit, sourceTree }));
}

export function collectSourceInventory({
  sourceRoot,
  fsApi = { readFileSync },
  runGit = defaultRunGit,
  expectedCommit = REVIEWED_SOURCE_COMMIT,
  expectedTree = null,
  sourceRef = "HEAD",
  reviewedPredicates = DEFAULT_REVIEWED_PREDICATES,
  claimedDispositions = [],
} = {}) {
  if (!sourceRoot || !isAbsolute(sourceRoot)) throw new Error("sourceRoot must be an absolute path");
  const source = {
    commit: runGit(["rev-parse", sourceRef], sourceRoot).trim(),
    tree: runGit(["rev-parse", `${sourceRef}^{tree}`], sourceRoot).trim(),
  };
  if (expectedCommit && source.commit !== expectedCommit) {
    throw new Error(`reviewed source commit mismatch: expected ${expectedCommit}, got ${source.commit}`);
  }
  if (expectedTree && source.tree !== expectedTree) {
    throw new Error(`reviewed source tree mismatch: expected ${expectedTree}, got ${source.tree}`);
  }
  const dirtyPaths = parseStatus(runGit(["status", "--porcelain=v1", "--untracked-files=all", "-z"], sourceRoot));
  const unexpectedDirtyPaths = dirtyPaths.filter((path) => !OWNED_DIRTY_PATHS.has(path));
  if (unexpectedDirtyPaths.length > 0) {
    throw new Error(`reviewed source has unexpected dirty paths: ${unexpectedDirtyPaths.sort().join(", ")}`);
  }
  assertDispositionClaims(claimedDispositions, reviewedPredicates, source);

  const files = collectTrackedFiles({ sourceRoot, sourceRef, fsApi, runGit });
  const entries = [];
  for (const file of files) {
    const lines = file.text.split(/\r?\n/);
    for (let lineIndex = 0; lineIndex < lines.length; lineIndex += 1) {
      const sourceLine = lines[lineIndex].trim();
      if (!sourceLine) continue;
      for (const spec of CATEGORY_SPECS) {
        for (const match of lineMatches(spec, lines[lineIndex])) {
          const finding = {
            category: spec.category,
            path: file.path,
            blob: file.blob,
            line: lineIndex + 1,
            column: match.index + 1,
            symbol: inferSymbol(lines, lineIndex),
            selector: redact(match.value),
            sourceLine,
            contextHash: sha256(sourceLine),
            affectedBehavior: spec.affectedBehavior,
            topology: inferTopology(file.path, lines, lineIndex),
            applicableMpIds: [...new Set(spec.mpIds)].sort(),
          };
          const matchingPredicates = reviewedPredicates.filter((predicate) => predicateMatches(finding, predicate, source));
          if (matchingPredicates.length > 1) throw new Error(`ambiguous reviewed predicate at ${file.path}:${lineIndex + 1}`);
          if (matchingPredicates.length === 1) {
            const predicate = matchingPredicates[0];
            if (!ALLOWED_DISPOSITIONS.has(predicate.disposition)) throw new Error(`reviewed predicate has forbidden disposition: ${predicate.id}`);
            finding.disposition = predicate.disposition;
            finding.applicableMpIds = [...new Set([...finding.applicableMpIds, ...predicate.applicableMpIds])].sort();
            finding.reviewedPredicateId = predicate.id;
          } else {
            finding.disposition = baseDisposition(finding);
          }
          delete finding.sourceLine;
          entries.push(finding);
        }
      }
    }
  }
  entries.sort((left, right) => JSON.stringify(left).localeCompare(JSON.stringify(right)));
  const observedCategories = [...new Set(entries.map((entry) => entry.category))].sort();
  const missingCategories = REQUIRED_CATEGORIES.filter((category) => !observedCategories.includes(category));
  const rowCoverage = buildRowCoverage(entries, source);
  const missingRows = rowCoverage.filter((row) => row.status === "missing").map((row) => row.id);
  const removalRequired = entries.filter((entry) => entry.disposition === "removal_required").length;
  const unreviewed = entries.filter((entry) => entry.disposition === "unreviewed").length;
  const report = {
    schema: INVENTORY_SCHEMA,
    source: {
      commit: source.commit,
      tree: source.tree,
      trackedFileCount: files.length,
    },
    rows: rowCoverage,
    requiredCategories: [...REQUIRED_CATEGORIES],
    observedCategories,
    missingCategories,
    missingRows,
    entries,
    summary: {
      entryCount: entries.length,
      removalRequired,
      unreviewed,
      allowedReleaseDeployment: entries.filter((entry) => entry.disposition === "allowed_release_deployment").length,
      requiredAutomaticShutdown: entries.filter((entry) => entry.disposition === "required_automatic_shutdown").length,
    },
    status: missingCategories.length === 0 && missingRows.length === 0 && removalRequired === 0 && unreviewed === 0 ? "pass" : "fail",
  };
  return stable(report);
}

function parseArgs(argv) {
  const options = {
    root: resolve(dirname(fileURLToPath(import.meta.url)), "../../.."),
    output: null,
    expectedCommit: REVIEWED_SOURCE_COMMIT,
    expectedTree: REVIEWED_SOURCE_TREE,
    sourceRef: REVIEWED_SOURCE_COMMIT,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--root") options.root = resolve(argv[++index]);
    else if (argument === "--output") options.output = resolve(argv[++index]);
    else if (argument === "--reviewed-commit") options.expectedCommit = argv[++index];
    else if (argument === "--reviewed-tree") options.expectedTree = argv[++index];
    else if (argument === "--source-ref") options.sourceRef = argv[++index];
    else if (argument === "--help") options.help = true;
    else throw new Error(`unknown argument: ${argument}`);
  }
  return options;
}

export function runCli(argv = process.argv.slice(2), io = { write: (value) => process.stdout.write(value), error: (value) => process.stderr.write(value) }) {
  const options = parseArgs(argv);
  if (options.help) {
    io.write("usage: node managed-parity-source-inventory.mjs [--root DIR] [--source-ref REF] [--reviewed-commit SHA] [--reviewed-tree TREE] [--output FILE]\n");
    return 0;
  }
  try {
    const report = collectSourceInventory({
      sourceRoot: options.root,
      sourceRef: options.sourceRef,
      expectedCommit: options.expectedCommit,
      expectedTree: options.expectedTree,
    });
    const output = stableJson(report);
    if (options.output) {
      writeFileSync(options.output, output, "utf8");
    } else {
      io.write(output);
    }
    return report.status === "pass" ? 0 : 1;
  } catch (error) {
    io.error(`${error instanceof Error ? error.message : String(error)}\n`);
    return 2;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) {
  const exitCode = runCli();
  process.exitCode = exitCode;
}
