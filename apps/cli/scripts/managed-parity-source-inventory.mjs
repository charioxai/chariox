#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, extname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { fragmentSourceViews, fragmentMatchAnchor } from "./lib/managed-parity-fragment-source.mjs";
import { stripCaddyComments } from "./lib/managed-parity-caddy-source.mjs";
import { stripPostgresComments } from "./lib/managed-parity-sql-source.mjs";
import { patchSourceViews } from "./lib/managed-parity-patch-source.mjs";
import { FROZEN_SEMANTIC_REVIEWS } from "./lib/managed-parity-semantic-reviews.mjs";
import { sourceRuleCandidates, sourceClassification, groupSourceClassifications, sourceAuditGaps } from "./lib/managed-parity-source-rules.mjs";
import { currentDeclarationCandidates } from "./lib/managed-parity-current-declarations.mjs";
import { CURRENT_SEMANTIC_REVIEWS } from "./lib/managed-parity-current-reviews.mjs";

export const INVENTORY_SCHEMA = "chariox.managed-parity.source-inventory.v2";
// These approvals are bound to a historical source only. They must not be
// rebound when the inventory runs against a newer checkout.
export const PRIOR_REVIEWED_SOURCE_COMMIT = "391b38b2be15c4f49d8ea70cc14b031385cf8331";
export const PRIOR_REVIEWED_SOURCE_TREE = "199b565b83e9582acd38a38a76520e2ba5ac02b4";
export const DEFAULT_SOURCE_REF = "HEAD";
const INVENTORY_TOOL_PATH = "apps/cli/scripts/managed-parity-source-inventory.mjs";
const INVENTORY_TOOL_MODULES = [
  ["./managed-parity-source-inventory.mjs", INVENTORY_TOOL_PATH],
  ["./lib/managed-parity-semantic-reviews.mjs", "apps/cli/scripts/lib/managed-parity-semantic-reviews.mjs"],
  ["./lib/managed-parity-patch-source.mjs", "apps/cli/scripts/lib/managed-parity-patch-source.mjs"],
  ["./lib/managed-parity-source-rules.mjs", "apps/cli/scripts/lib/managed-parity-source-rules.mjs"],
  ["./lib/managed-parity-current-declarations.mjs", "apps/cli/scripts/lib/managed-parity-current-declarations.mjs"],
  ["./lib/managed-parity-current-reviews.mjs", "apps/cli/scripts/lib/managed-parity-current-reviews.mjs"],
  ["./lib/managed-parity-fragment-source.mjs", "apps/cli/scripts/lib/managed-parity-fragment-source.mjs"],
  ["./lib/managed-parity-sql-source.mjs", "apps/cli/scripts/lib/managed-parity-sql-source.mjs"],
  ["./lib/managed-parity-caddy-source.mjs", "apps/cli/scripts/lib/managed-parity-caddy-source.mjs"],
];

function inventoryToolIdentity() {
  const modules = INVENTORY_TOOL_MODULES.map(([relative, path]) => ({
    path, sha256: sha256(readFileSync(fileURLToPath(new URL(relative, import.meta.url)))),
  }));
  return { path: INVENTORY_TOOL_PATH, sha256: modules[0].sha256,
    modules, bundleSha256: sha256(stableJson(modules)) };
}

export const REQUIRED_CATEGORIES = Object.freeze([
  "managed_only_branch",
  "managed_env_selector",
  "kernel_slice_broker_control",
  "bubblewrap",
  "managed_service_restriction",
  "protected_path_filter",
  "managed_only_error_mapping",
  "repository_home_state_path",
  "client_projection",
  "cleanup_selector",
  "release_activation",
  "release_artifact_exporter",
  "automatic_shutdown_selector",
]);

export const MP_ROWS = Object.freeze([
  {
    id: "MP-01",
    name: "remove Path-1 Bubblewrap and inherited managed sandboxing",
    categories: ["managed_env_selector", "kernel_slice_broker_control", "bubblewrap", "managed_service_restriction"],
  },
  {
    id: "MP-02",
    name: "ordinary directory discovery, exact-path entry, and provider access",
    categories: ["repository_home_state_path", "protected_path_filter"],
  },
  {
    id: "MP-03",
    name: "protect exact managed control state without blocking siblings",
    categories: ["protected_path_filter", "kernel_slice_broker_control"],
  },
  {
    id: "MP-04",
    name: "ordinary HOME and CHARIOX_HOME state layout",
    categories: ["repository_home_state_path"],
  },
  {
    id: "MP-05",
    name: "source-basename repository materialization and collision safety",
    categories: ["repository_home_state_path"],
  },
  {
    id: "MP-06",
    name: "server-authoritative custom repository root",
    categories: ["repository_home_state_path", "client_projection"],
  },
  {
    id: "MP-07",
    name: "signed content-addressed release activation",
    categories: ["release_activation", "release_artifact_exporter"],
  },
  {
    id: "MP-08",
    name: "ordinary kernel runtime, protocol, adapters, state, and clients",
    categories: ["managed_only_branch", "kernel_slice_broker_control", "managed_only_error_mapping", "client_projection", "cleanup_selector"],
  },
  {
    id: "MP-09",
    name: "mandatory managed automatic-shutdown lifecycle",
    categories: ["automatic_shutdown_selector"],
  },
  {
    id: "MP-10",
    name: "fresh-machine ordinary-versus-managed comparison and cleanup evidence",
    categories: ["source_identity", "cleanup_selector"],
  },
  {
    id: "MP-11",
    name: "proactive managed-only source inventory",
    categories: [...REQUIRED_CATEGORIES],
  },
]);

const SCANNABLE_EXTENSIONS = new Set([
  ".apparmor",
  ".awk",
  ".c",
  ".cc",
  ".h",
  ".caddyfile",
  ".bash",
  ".cjs",
  ".conf",
  ".container",
  ".dockerfile",
  ".env",
  ".fish",
  ".js",
  ".jsx",
  ".json",
  ".mjs",
  ".mjsfrag",
  ".mount",
  ".network",
  ".path",
  ".patch",
  ".plist",
  ".policy",
  ".profile",
  ".py",
  ".rs",
  ".seccomp",
  ".service",
  ".sh",
  ".sql",
  ".socket",
  ".swift",
  ".target",
  ".timer",
  ".toml",
  ".ts",
  ".tsx",
  ".tsfrag",
  ".yaml",
  ".yml",
  ".zsh",
]);

const SCANNABLE_EXTENSIONLESS_BASENAMES = new Set([
  "Containerfile",
  "Dockerfile",
  "Makefile",
]);

const IGNORED_DIRECTORY_NAMES = new Set([
  ".git",
  ".next",
  ".turbo",
  "build",
  "codegen",
  "coverage",
  "dist",
  "generated",
  "node_modules",
  "target",
]);

const IGNORED_PATH_PARTS = [
  /(?:^|\/)docs(?:\/|$)/i,
  /(?:^|\/)(?:fixtures?|snapshots?)(?:\/|$)/i,
  /(?:^|\/)(?:autogen|codegen|generated|__generated__)(?:\/|$)/i,
  /(?:^|\/)[^/]*(?:\.generated|\.autogen|\.gen)\.[^/]+$/i,
];

const PRODUCTION_PATH_PREFIXES = [
  "apps/app-worker/",
  "apps/app-storage-helper/",
  "adapters/",
  "apps/cli/",
  "apps/ios/",
  "apps/api/",
  "apps/admin/",
  "apps/web/",
  "apps/worker/",
  "apps/infrastructure-manager/",
  "apps/kernel/",
  "connector-adapters/",
  "deploy/",
  "docker/",
  "packages/",
  "scripts/",
];

// MP-11: these are exact non-executable synthetic fixtures, not source formats.
// Changed contents/mode require a fresh exclusion decision, never inheritance.
const EXCLUDED_FIXTURE_BLOBS = Object.freeze({
  "apps/kernel/src/runtime/state/testdata/claude-setup-token-2.1.281-start.pty": "d95a0f81eeed8d1ed7d7c015148db4ae94fcecba",
  "packages/app-package/fuzz/seeds/archive/0": "26bc3530acf221ec9be6f0aec5dac3afa24ea72c",
  "packages/app-package/fuzz/seeds/signed/0": "cc0d165b5ec9e865402a8192fce4003e3a0095a4",
  "packages/app-package/fuzz/seeds/signed/1": "d8313ed338ac58bed4f958eed6ed672edc6daabd",
  "packages/app-package/fuzz/seeds/signed/2": "c7e62e2614e5743c93982528642ae423805099c9",
  "packages/app-package/fuzz/seeds/signed/3": "1fad10dc55a0499ba151d40adfc82277fa5f771d",
});

// These are tracked repository assets, documentation, native build metadata,
// or foreign-language implementation files. They are intentionally outside
// this MP source inventory; a new production suffix is not silently ignored.
const NON_INVENTORIED_PRODUCTION_EXTENSIONS = new Set([
  ".chariox",
  ".charioxignore",
  ".css",
  ".dockerignore",
  ".entitlements",
  ".example",
  ".gitignore",
  ".gitkeep",
  ".html",
  ".license",
  ".lock",
  ".md",
  ".mdc",
  ".pbxproj",
  ".prisma",
  ".rst",
  ".svg",
  ".swiftformat",
  ".txt",
  ".xcodeproj",
  ".xcconfig",
  ".xcscheme",
  ".xctestplan",
  ".xcworkspacedata",
]);

const NON_INVENTORIED_PRODUCTION_BASENAMES = new Set([
  "tint2rc",
]);

const CONTAINER_FILE_RE = /(?:^|\/)(?:Dockerfile|Containerfile)(?:\.[^/]*)?$/i;
const COMPOSE_FILE_RE = /(?:^|\/)(?:docker-compose|compose)(?:\.[^/]*)?$/i;

const CATEGORY_SPECS = Object.freeze([
  {
    category: "kernel_slice_broker_control",
    mpIds: ["MP-01", "MP-03", "MP-08", "MP-11"],
    affectedBehavior: "kernel-only slice broker scope, lease transport, or required-lease marker",
    pattern: /\bCHARIOX_SLICE_(?:ROOT|DOCKER_BROKER_(?:SOCKET|FD|REQUIRED))\b/g,
  },
  {
    category: "managed_env_selector",
    mpIds: ["MP-01", "MP-08", "MP-11"],
    affectedBehavior: "managed environment selector or injected managed runtime marker",
    pattern: /\b(?:CHARIOX_OPENSHIP_(?:RUNTIME_PROFILE|NETWORK_MODE|CONTAINER_NAME|READ_ONLY_BINDS_JSON)|OPENSHIP_CHARIOX_(?:COLOCATION_SECRET|INSTANCE_ID)|CHARIOX_MANAGED[A-Z0-9_]*|CHARIOX_DISPOSABLE_WORKER_[A-Z0-9_]*|CHARIOX_PUBLICATION_CONTROL_[A-Z0-9_]*|CHARIOX_WORKER_ISOLATION_[A-Z0-9_]*)\b/g,
  },
  {
    category: "bubblewrap",
    mpIds: ["MP-01", "MP-11"],
    affectedBehavior: "Bubblewrap or equivalent provider ancestry/sandbox boundary",
    pattern: /(?:\/usr\/bin\/)?\bbwrap\b|\bbubblewrap\b/gi,
  },
  {
    category: "managed_service_restriction",
    mpIds: ["MP-01", "MP-08", "MP-11"],
    affectedBehavior: "managed service-unit or inherited system restriction",
    pattern: /\b(?:ProtectSystem|ProtectHome|PrivateTmp|NoNewPrivileges|Restrict[A-Za-z0-9_]*|ReadWritePaths|UMask)\b/g,
  },
  {
    category: "managed_only_branch",
    mpIds: ["MP-08", "MP-11"],
    affectedBehavior: "branch or mode switch that changes managed runtime behavior",
    pattern: /\b(?:is_managed|managed_mode|managed_only|managed_context|managedEnvironment|managedRuntime)\b|\b(?:if|else\s+if|match|switch|case)\b[^\n;{}]{0,120}\bmanaged\b/gi,
  },
  {
    category: "protected_path_filter",
    mpIds: ["MP-02", "MP-03", "MP-11"],
    affectedBehavior: "protected/control path or parent filtering",
    pattern: /\b(?:protected[_ -]?paths?|protected_parent|protected_path|control[_ -]?(?:root|file|path)|is_protected|force_excludes?|path_filter)\b/gi,
  },
  {
    category: "managed_only_error_mapping",
    mpIds: ["MP-08", "MP-11"],
    affectedBehavior: "managed-only error, failure, denial, or unsupported mapping",
    pattern: /\bmanaged\b[^\n;{}]{0,100}\b(?:error|failure|failed|denied|unsupported|invalid)\b|\b(?:error|failure|failed|denied|unsupported|invalid)\b[^\n;{}]{0,100}\bmanaged\b/gi,
  },
  {
    category: "repository_home_state_path",
    mpIds: ["MP-02", "MP-04", "MP-05", "MP-06", "MP-11"],
    affectedBehavior: "repository-root, home, provider-home, or durable state path",
    pattern: /\b(?:CHARIOX_HOME|CHARIOX_MANAGED_REPOSITORY_ROOT|managed[_-]?repository[_-]?root|repository[_-]?root|state[_-]?root|provider[_-]?home)\b|\/home\/chariox(?:\/[A-Za-z0-9._/-]+)?|\/var\/lib\/chariox(?:\/[A-Za-z0-9._/-]+)?|\.chariox\b/g,
  },
  {
    category: "client_projection",
    mpIds: ["MP-06", "MP-08", "MP-11"],
    affectedBehavior: "managed state projected to a client, response, UI, or result",
    pattern: /\bmanaged\b[^\n;{}]{0,100}\b(?:projection|client|response|payload|result|json|ui|web|history|prompt)\b|\b(?:projection|client|response|payload|result|json|ui|web|history|prompt)\b[^\n;{}]{0,100}\bmanaged\b/gi,
  },
  {
    category: "cleanup_selector",
    mpIds: ["MP-08", "MP-10", "MP-11"],
    affectedBehavior: "managed cleanup, revocation, expiry, reaping, or deletion path",
    pattern: /\bmanaged\b[^\n;{}]{0,100}\b(?:cleanup|revoke|revocation|expire|expiry|reap|delete|dispose)\b|\b(?:cleanup|revoke|revocation|expire|expiry|reap|delete|dispose)\b[^\n;{}]{0,100}\bmanaged\b/gi,
  },
  {
    category: "release_activation",
    mpIds: ["MP-07", "MP-11"],
    affectedBehavior: "signed managed release activation and artifact identity",
    pattern: /\b(?:verify_release|release[-_ ]manifest|release[-_ ]signature|release[-_]public[-_]key|signed release|release_digest|kernel artifact)\b/gi,
  },
  {
    category: "release_artifact_exporter",
    mpIds: ["MP-07", "MP-11"],
    affectedBehavior: "managed image stage exporting only native release artifacts",
    pattern: /\bFROM\s+scratch\s+AS\s+managed-release-artifacts\b[^\n]*|\bCOPY\s+--from=rust-builder\s+\/opt\/chariox-source\/target\/release\/chariox-(?:kernel|managed-bootstrap|relay)\s+\/chariox-(?:kernel|managed-bootstrap|relay)\b[^\n]*/gi,
  },
  {
    category: "automatic_shutdown_selector",
    mpIds: ["MP-09", "MP-11"],
    affectedBehavior: "managed automatic shutdown policy or trigger selector",
    pattern: /\b(?:auto[_-]?stop|automatic[_-]?shutdown|idle[_-]?delay|minimum[_-]?runtime|keep[_-]?running|last[_-]?agent|agents[_-]?done)(?:[_-][A-Za-z0-9]+)*\b|\b(?:managed|idle|minimum|deployment|lifecycle)\b[^\n;{}]{0,100}\bshutdown\b/gi,
  },
]);

// Preserve these historical predicate identities unchanged. They remain
// useful audit anchors but are not semantic approvals without independent
// review metadata and cannot classify candidates as allowed.
export const DEFAULT_REVIEWED_PREDICATES = Object.freeze([
  {
    id: "MP-07-release-verify-release",
    sourceCommit: PRIOR_REVIEWED_SOURCE_COMMIT,
    sourceTree: PRIOR_REVIEWED_SOURCE_TREE,
    path: "apps/kernel/src/managed_bootstrap/release.rs",
    line: 39,
    symbol: "verify_release",
    sourceLine: "pub(super) fn verify_release(",
    category: "release_activation",
    topology: "direct_path1",
    applicableMpIds: ["MP-07", "MP-11"],
    disposition: "allowed_release_deployment",
  },
  {
    id: "MP-09-auto-stop-policy",
    sourceCommit: PRIOR_REVIEWED_SOURCE_COMMIT,
    sourceTree: PRIOR_REVIEWED_SOURCE_TREE,
    path: "apps/kernel/src/runtime/managed_environment_control/cloud_contract.rs",
    line: 110,
    symbol: "AutoStopPolicy",
    sourceLine: "minimum_runtime_seconds: u64,",
    category: "automatic_shutdown_selector",
    topology: "direct_path1",
    applicableMpIds: ["MP-09", "MP-11"],
    disposition: "required_automatic_shutdown",
  },
  {
    id: "MP-09-auto-stop-idle-delay",
    sourceCommit: PRIOR_REVIEWED_SOURCE_COMMIT,
    sourceTree: PRIOR_REVIEWED_SOURCE_TREE,
    path: "apps/kernel/src/runtime/managed_environment_control/cloud_contract.rs",
    line: 111,
    symbol: "AutoStopPolicy",
    sourceLine: "idle_delay_seconds: Option<u64>,",
    category: "automatic_shutdown_selector",
    topology: "direct_path1",
    applicableMpIds: ["MP-09", "MP-11"],
    disposition: "required_automatic_shutdown",
  },
]);

const ALLOWED_DISPOSITIONS = new Set([
  "allowed_release_deployment",
  "required_automatic_shutdown",
]);

const SEMANTIC_DISPOSITIONS = new Set([
  ...ALLOWED_DISPOSITIONS,
  "removal_required",
  "negative_guard",
  "test_evidence",
  "ordinary_path1_behavior",
  "shared_host_isolation",
  "inner_docker_slice_isolation",
]);

// Historical records above preserve the exact prior review identities. They
// are not semantic approvals because they contain no independent reviewer
// metadata. New approvals must bind the complete current source/candidate
// anchor and the review that authorized the classification.
export const DEFAULT_SEMANTIC_DISPOSITIONS = Object.freeze([...FROZEN_SEMANTIC_REVIEWS, ...CURRENT_SEMANTIC_REVIEWS]);

const OWNED_DIRTY_PATHS = new Set([
  "apps/cli/scripts/lib/managed-parity-current-reviews.mjs",
  "apps/cli/scripts/lib/managed-parity-current-declarations.mjs",
  "apps/cli/scripts/lib/managed-parity-semantic-reviews.mjs",
  "apps/cli/scripts/lib/managed-parity-patch-source.mjs",
  "apps/cli/scripts/lib/managed-parity-source-rules.mjs",
  "apps/cli/scripts/lib/managed-parity-fragment-source.mjs",
  "apps/cli/scripts/lib/managed-parity-sql-source.mjs",
  "apps/cli/scripts/lib/managed-parity-caddy-source.mjs",
  "apps/cli/scripts/managed-parity-source-inventory.mjs",
  "apps/cli/scripts/managed-parity-source-inventory.test.mjs",
  "docs/MANAGED_PATH1_PARITY_INVENTORY.md",
]);

const SELF_EXCLUDED_PATHS = new Set([
  "apps/cli/scripts/lib/managed-parity-current-reviews.mjs",
  "apps/cli/scripts/lib/managed-parity-current-declarations.mjs",
  "apps/cli/scripts/lib/managed-parity-semantic-reviews.mjs",
  "apps/cli/scripts/lib/managed-parity-patch-source.mjs",
  "apps/cli/scripts/lib/managed-parity-source-rules.mjs",
  "apps/cli/scripts/lib/managed-parity-fragment-source.mjs",
  "apps/cli/scripts/lib/managed-parity-sql-source.mjs",
  "apps/cli/scripts/lib/managed-parity-caddy-source.mjs",
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
    .replace(/[\u0000-\u001f]/g, " ")
    .replace(/[\u007f-\u009f]/g, " ")
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

export function parseStatus(output) {
  const records = output.split("\0");
  const paths = [];
  for (let index = 0; index < records.length; index += 1) {
    const record = records[index];
    if (!record) continue;
    paths.push(record.slice(3));
    if (/[RC]/.test(record.slice(0, 2))) {
      const originalPath = records[++index];
      if (!originalPath) throw new Error("incomplete git status rename/copy record");
      paths.push(originalPath);
    }
  }
  return paths;
}

function isIgnoredPath(path) {
  return SELF_EXCLUDED_PATHS.has(path) || IGNORED_PATH_PARTS.some((pattern) => pattern.test(path));
}

function isProductionPath(path) {
  return PRODUCTION_PATH_PREFIXES.some((prefix) => path.startsWith(prefix));
}

export function classifyProductionPath(path, requireKnownFormat = false) {
  if (isIgnoredPath(path)) return null;
  // MP-11: six exact synthetic/non-executable fixtures, never a directory exemption.
  if (Object.hasOwn(EXCLUDED_FIXTURE_BLOBS, path)) {
    if (requireKnownFormat) throw new Error(`unsupported patch fixture content: ${path}`);
    return null;
  }
  const parts = path.split("/");
  if (parts.some((part) => IGNORED_DIRECTORY_NAMES.has(part))) return null;
  const fileName = basename(path);
  const lowerPath = path.toLowerCase();
  const extension = extname(path).toLowerCase();
  if (CONTAINER_FILE_RE.test(path) || COMPOSE_FILE_RE.test(path)) return "container";
  if (SCANNABLE_EXTENSIONLESS_BASENAMES.has(fileName)) return fileName === "Makefile" ? "shell" : "container";
  if (fileName === "chariox-open-url" || path === "scripts/macos-pkg/postinstall") return "shell";
  if (fileName === "tint2rc") return "config";
  if (path.endsWith(".service.in")) return "unit";
  if (!SCANNABLE_EXTENSIONS.has(extension)) {
    if (!isProductionPath(path) && !requireKnownFormat) return null;
    if (NON_INVENTORIED_PRODUCTION_BASENAMES.has(fileName)
      || NON_INVENTORIED_PRODUCTION_EXTENSIONS.has(extension)
      // extname(".gitignore") is empty, so match only exact dotfile names
      // already listed in the reviewed non-inventoried extension set.
      || (fileName.startsWith(".") && NON_INVENTORIED_PRODUCTION_EXTENSIONS.has(fileName))) return null;
    throw new Error(`unclassified production file: ${path}`);
  }
  if (extension === ".patch") return "patch";
  if ([".rs"].includes(extension)) return "rust";
  if ([".c", ".cc", ".h"].includes(extension)) return "c";
  if (extension === ".sql") return "sql";
  if (extension === ".caddyfile") return "caddy";
  if (extension === ".py") return "python";
  if ([".js", ".jsx", ".mjs", ".mjsfrag", ".cjs", ".ts", ".tsx", ".tsfrag"].includes(extension)) return "javascript";
  if (extension === ".swift") return "swift";
  if ([".sh", ".bash", ".zsh", ".fish", ".awk"].includes(extension)) return "shell";
  if (extension === ".dockerfile") return "container";
  if ([".service", ".socket", ".mount", ".network", ".path", ".timer", ".target", ".container"].includes(extension)) return "unit";
  if ([".apparmor", ".policy", ".profile", ".seccomp"].includes(extension)
    || lowerPath.includes("apparmor")) return "policy";
  return "config";
}

function stripComments(text, format) {
  if (format === "sql") return stripPostgresComments(text);
  if (format === "caddy") return stripCaddyComments(text);
  const slashComments = ["c", "rust", "javascript", "swift"].includes(format);
  const hashComments = ["python", "shell", "unit", "container", "policy", "config"].includes(format);
  if (!slashComments && !hashComments) return text;
  const output = text.split("");
  let blockComment = false;
  let quote = null;
  let escaped = false;
  for (let index = 0; index < output.length; index += 1) {
    const current = text[index];
    const next = text[index + 1];
    if (blockComment) {
      if (current === "*" && next === "/") {
        output[index] = " ";
        output[index + 1] = " ";
        index += 1;
        blockComment = false;
      } else if (current !== "\n" && current !== "\r") {
        output[index] = " ";
      }
      continue;
    }
    if (quote) {
      if (escaped) {
        escaped = false;
      } else if (current === "\\") {
        escaped = true;
      } else if (current === quote) {
        quote = null;
      }
      continue;
    }
    if (current === "\"" || current === "'" || (format === "javascript" && current === "`")) {
      quote = current;
      continue;
    }
    if (slashComments && current === "/" && next === "*") {
      output[index] = " ";
      output[index + 1] = " ";
      index += 1;
      blockComment = true;
      continue;
    }
    if (slashComments && current === "/" && next === "/") {
      while (index < output.length && text[index] !== "\n" && text[index] !== "\r") {
        output[index] = " ";
        index += 1;
      }
      index -= 1;
      continue;
    }
    if (hashComments && current === "#") {
      while (index < output.length && text[index] !== "\n" && text[index] !== "\r") {
        output[index] = " ";
        index += 1;
      }
      index -= 1;
    }
  }
  return output.join("");
}

function readText(fsApi, absolutePath) {
  const value = fsApi.readFileSync(absolutePath);
  if (value.includes(0)) throw new Error(`unsupported binary source: ${absolutePath}`);
  return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(value);
}

function inferSymbol(lines, lineIndex) {
  for (let index = lineIndex; index >= Math.max(0, lineIndex - 12); index -= 1) {
    const line = lines[index];
    const match = /\b(?:fn|function|class|struct|enum|const|static|impl|mod)\s+([A-Za-z0-9_:-]+)/.exec(line);
    if (match) return match[1].replace(/:+$/, "");
  }
  return null;
}

function isTestSourcePath(path) {
  return /(?:^|\/)(?:test|tests|__tests__)(?:\/|$)/i.test(path)
    || /(?:^|\/)(?:[^/]+\.(?:test|spec)|[^/]+_(?:test|spec)|test_[^/]+|tests?)\.[^/]+$/i.test(path);
}

function maskRustLiterals(text) {
  const output = text.split("");
  let index = 0;
  while (index < text.length) {
    const rawStart = /^(?:b)?r(#+)?"/.exec(text.slice(index));
    if (rawStart) {
      const hashes = rawStart[1] ?? "";
      const terminator = `"${hashes}`;
      const end = text.indexOf(terminator, index + rawStart[0].length);
      const stop = end < 0 ? text.length : end + terminator.length;
      for (let cursor = index; cursor < stop; cursor += 1) {
        if (text[cursor] !== "\n" && text[cursor] !== "\r") output[cursor] = " ";
      }
      index = stop;
      continue;
    }
    if (text[index] === '"') {
      let cursor = index + 1;
      while (cursor < text.length) {
        if (text[cursor] === "\\") cursor += 2;
        else if (text[cursor] === '"') {
          cursor += 1;
          break;
        } else cursor += 1;
      }
      for (let position = index; position < Math.min(cursor, text.length); position += 1) {
        if (text[position] !== "\n" && text[position] !== "\r") output[position] = " ";
      }
      index = cursor;
      continue;
    }
    const charLiteral = /^'(?:\\.|[^'\\])'/.exec(text.slice(index));
    if (charLiteral) {
      for (let position = index; position < index + charLiteral[0].length; position += 1) {
        if (text[position] !== "\n" && text[position] !== "\r") output[position] = " ";
      }
      index += charLiteral[0].length;
      continue;
    }
    index += 1;
  }
  return output.join("");
}

function rustBlockEnd(codeLines, startLine) {
  let depth = 0;
  let opened = false;
  for (let lineIndex = startLine; lineIndex < codeLines.length; lineIndex += 1) {
    for (const character of codeLines[lineIndex]) {
      if (character === "{") {
        opened = true;
        depth += 1;
      } else if (character === "}" && opened) {
        depth -= 1;
        if (depth === 0) return lineIndex;
      }
    }
  }
  return codeLines.length - 1;
}

function rustTestDeclarationEnd(codeLines, declarationLine) {
  if (!/\bmod\s+[A-Za-z_][A-Za-z0-9_]*\b/.test(codeLines[declarationLine])) {
    return rustBlockEnd(codeLines, declarationLine);
  }
  // This input has comments and literals masked. A semicolon that arrives
  // before any body brace terminates an out-of-line module declaration.
  for (let lineIndex = declarationLine; lineIndex < codeLines.length; lineIndex += 1) {
    for (const character of codeLines[lineIndex]) {
      if (character === ";") return lineIndex;
      if (character === "{") return rustBlockEnd(codeLines, declarationLine);
    }
  }
  return declarationLine;
}

function findRustTestRanges(text, path) {
  const lines = stripComments(text, "rust").split(/\r?\n/);
  const codeLines = maskRustLiterals(stripComments(text, "rust")).split(/\r?\n/);
  const ranges = [];
  if (isTestSourcePath(path)) ranges.push({ start: 0, end: lines.length - 1, kind: "test_source" });
  for (let index = 0; index < lines.length; index += 1) {
    const marker = /^\s*#\s*\[\s*(?:cfg\s*\(\s*test\s*\)|test)\s*\]\s*$/.test(lines[index]);
    if (!marker) continue;
    let declaration = index + 1;
    while (declaration < lines.length && declaration <= index + 12) {
      const candidate = lines[declaration].trim();
      if (candidate && !candidate.startsWith("#")) break;
      declaration += 1;
    }
    if (declaration >= lines.length || declaration > index + 12) continue;
    if (!/\b(?:mod|fn)\s+[A-Za-z_][A-Za-z0-9_]*\b/.test(lines[declaration])) continue;
    ranges.push({ start: index, end: rustTestDeclarationEnd(codeLines, declaration), kind: "cfg_test" });
  }
  return ranges;
}

function explicitUnitTopology(path, text) {
  if (!path.endsWith(".service") && !path.endsWith(".service.in")) return "not_service_unit";
  const values = [...text.matchAll(/^\s*Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=(path1|shared_host)\s*$/gm)]
    .map((match) => match[1]);
  if (values.length === 1) return values[0];
  if (values.length > 1) return "ambiguous";
  return "unresolved";
}

function isVerificationGuardCandidate(path, lines, lineIndex, format) {
  if (format !== "javascript" || !/verify|release/i.test(path)) return false;
  const nearby = lines.slice(Math.max(0, lineIndex - 48), Math.min(lines.length, lineIndex + 49)).join(" ");
  return /\bforbidden\b/.test(nearby)
    && /\.includes\s*\(/.test(nearby)
    && /\b(?:fail|reject|throw)\s*\(/.test(nearby);
}

function inferSourceRoleHints({ path, format, lines, lineIndex, testRanges, unitTopology, category }) {
  const inTestRegion = isTestSourcePath(path)
    || testRanges.some((range) => lineIndex >= range.start && lineIndex <= range.end);
  const verificationGuard = isVerificationGuardCandidate(path, lines, lineIndex, format);
  const innerSlice = path.startsWith("apps/kernel/slice-linux-docker/")
    || path.includes("/slice/local_docker/");
  const topology = format === "unit" ? unitTopology
    : innerSlice ? "inner_docker_slice" : "unresolved";
  let executionRole = "unresolved_source_role";
  if (inTestRegion) executionRole = "test_evidence_candidate";
  else if (verificationGuard) executionRole = "verification_guard_candidate";
  else if (format === "unit" && /^\s*Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=/.test(lines[lineIndex])) {
    executionRole = "selected_service_topology_marker";
  } else if (format === "unit" && topology === "path1") executionRole = "path1_service_directive_candidate";
  else if (format === "unit" && topology === "shared_host") executionRole = "shared_host_service_directive_candidate";
  else if (innerSlice) executionRole = "inner_docker_slice_configuration_candidate";

  const positivePath1Directive = format === "unit"
    && topology === "path1"
    && ["bubblewrap", "managed_service_restriction"].includes(category);
  return {
    executionRole,
    topologyHint: topology,
    testRegion: inTestRegion ? (isTestSourcePath(path) ? "test_source" : "cfg_test") : "none_detected",
    verificationGuardCandidate: verificationGuard,
    selectedServiceTopology: topology === "path1" || topology === "shared_host" ? topology : null,
    positivePath1Directive,
    authoritative: false,
  };
}

function lineMatches(spec, line) {
  spec.pattern.lastIndex = 0;
  return [...line.matchAll(spec.pattern)].map((match) => ({ value: match[0], index: match.index ?? 0 }));
}

function historicalPredicateMatches(finding, predicate, source) {
  return predicate.sourceCommit === source.commit
    && predicate.sourceTree === source.tree
    && predicate.path === finding.path
    && predicate.line === finding.line
    && predicate.symbol === finding.symbol
    && predicate.category === finding.category
    && predicate.sourceLine === finding.sourceLine;
}

function candidateAnchor(candidate) {
  return {
    path: candidate.path,
    blob: candidate.blob,
    line: candidate.line,
    column: candidate.column,
    symbol: candidate.symbol,
    category: candidate.category,
    selector: candidate.selector,
    contextHash: candidate.contextHash,
  };
}

function validateSemanticReview(review) {
  const anchor = review?.anchor;
  const metadata = review?.independentReview;
  const validSha = (value) => typeof value === "string" && /^[0-9a-f]{40}$/.test(value);
  if (typeof review?.id !== "string" || !review.id.trim()
    || !validSha(review.sourceCommit) || !validSha(review.sourceTree)
    || !anchor || typeof anchor.path !== "string" || !anchor.path
    || !validSha(anchor.blob) || !Number.isInteger(anchor.line) || anchor.line < 1
    || !Number.isInteger(anchor.column) || anchor.column < 1
    || !(anchor.symbol === null || (typeof anchor.symbol === "string" && anchor.symbol))
    || !REQUIRED_CATEGORIES.includes(anchor.category)
    || typeof anchor.selector !== "string" || !anchor.selector
    || !/^[0-9a-f]{64}$/.test(anchor.contextHash ?? "")
    || !SEMANTIC_DISPOSITIONS.has(review.disposition)
    || metadata?.independent !== true
    || typeof metadata.reviewer !== "string" || !metadata.reviewer.trim()
    || typeof metadata.reviewId !== "string" || !metadata.reviewId.trim()
    || typeof metadata.reviewedAt !== "string" || !Number.isFinite(Date.parse(metadata.reviewedAt))
    || typeof metadata.rationale !== "string" || metadata.rationale.trim().length < 20) {
    throw new Error(`invalid independent semantic review metadata: ${review?.id ?? "unknown"}`);
  }
}

function semanticReviewKey(candidate, source) {
  return stableJson({
    sourceCommit: source.commit,
    sourceTree: source.tree,
    anchor: candidateAnchor(candidate),
  });
}

function buildSemanticReviewIndex(reviews) {
  const index = new Map();
  const approvalIds = new Set();
  const reviewIds = new Set();
  for (const review of reviews) {
    validateSemanticReview(review);
    if (approvalIds.has(review.id) || reviewIds.has(review.independentReview.reviewId)) {
      throw new Error(`duplicate independent semantic review identity: ${review.id}`);
    }
    approvalIds.add(review.id);
    reviewIds.add(review.independentReview.reviewId);
    const key = stableJson({
      sourceCommit: review.sourceCommit,
      sourceTree: review.sourceTree,
      anchor: review.anchor,
    });
    index.set(key, [...(index.get(key) ?? []), review]);
  }
  return index;
}

function resolveSemanticDisposition(candidate, source, reviewIndex) {
  const matching = reviewIndex.get(semanticReviewKey(candidate, source)) ?? [];
  if (matching.length > 1) throw new Error(`ambiguous independent semantic reviews for ${candidate.path}:${candidate.line}`);
  if (matching.length === 0) return { status: "unreviewed", disposition: null, gateEffect: "fail_closed" };
  const review = matching[0];
  return {
    status: "reviewed",
    disposition: review.disposition,
    gateEffect: review.disposition === "removal_required" ? "fail" : "reviewed",
    reviewId: review.independentReview.reviewId,
    reviewer: review.independentReview.reviewer,
  };
}

// Exposed for focused contract tests. The inventory itself only builds this
// index from its checked-in review records; caller claims never feed it.
export function evaluateSemanticDisposition(candidate, source, reviews) {
  return resolveSemanticDisposition(candidate, source, buildSemanticReviewIndex(reviews));
}

function assertDispositionClaims(claims, reviews, source) {
  for (const claim of claims ?? []) {
    const review = reviews.find((candidate) => candidate.id === claim?.id);
    if (!review || review.sourceCommit !== source.commit || review.sourceTree !== source.tree
      || stableJson(review) !== stableJson(claim)) {
      throw new Error(`unverified independent semantic disposition claim: ${claim?.id ?? "unknown"}`);
    }
  }
}

function buildRowCoverage(entries) {
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
  for (const entry of treeEntries) {
    if (Object.hasOwn(EXCLUDED_FIXTURE_BLOBS, entry.path)
      && (entry.mode !== "100644" || entry.blob !== EXCLUDED_FIXTURE_BLOBS[entry.path])) {
      throw new Error(`excluded fixture source drift: ${entry.path}`);
    }
  }
  const selected = treeEntries.map((entry) => ({ ...entry, format: classifyProductionPath(entry.path) }))
    .filter((entry) => entry.format);
  for (const entry of selected) {
    if (!["100644", "100755"].includes(entry.mode)) throw new Error(`unsupported source mode: ${entry.path}`);
  }
  const blobs = new Map();
  if (runGit === defaultRunGit && selected.length) {
    // MP-11: read immutable objects even for HEAD. A mutable working copy
    // must never be attributed to an unchanged committed blob.
    const batch = spawnSync("git", ["cat-file", "--batch"], { cwd: sourceRoot,
      input: selected.map((entry) => entry.blob).join("\n") + "\n", maxBuffer: 256 * 1024 * 1024 });
    if (batch.error || batch.status !== 0) throw batch.error ?? new Error("Git blob batch failed");
    let cursor = 0;
    for (const entry of selected) {
      const end = batch.stdout.indexOf(10, cursor);
      const header = batch.stdout.subarray(cursor, end).toString("ascii").split(" ");
      const size = Number(header[2]);
      if (end < cursor || header[0] !== entry.blob || header[1] !== "blob"
        || !Number.isSafeInteger(size) || size < 0 || end + size + 1 >= batch.stdout.length
        || batch.stdout[end + size + 1] !== 10) throw new Error("Git batch blob identity/framing mismatch");
      const bytes = batch.stdout.subarray(end + 1, end + 1 + size);
      if (bytes.includes(0)) throw new Error(`unsupported binary source: ${entry.path}`);
      blobs.set(entry.blob, new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes));
      cursor = end + size + 2;
    }
    if (cursor !== batch.stdout.length) throw new Error("Git batch trailing data");
  }
  return selected.map((entry) => {
    return {
      ...entry,
      text: blobs.has(entry.blob) ? blobs.get(entry.blob) : sourceRef === "HEAD"
        ? readText(fsApi, join(sourceRoot, entry.path))
        : runGit(["show", `${sourceRef}:${entry.path}`], sourceRoot),
    };
  });
}

export function collectSourceInventory({
  sourceRoot,
  fsApi = { readFileSync },
  runGit = defaultRunGit,
  expectedCommit = null,
  expectedTree = null,
  sourceRef = DEFAULT_SOURCE_REF,
  claimedDispositions = [],
} = {}) {
  if (!sourceRoot || !isAbsolute(sourceRoot)) throw new Error("sourceRoot must be an absolute path");
  const source = {
    ref: sourceRef,
    commit: runGit(["rev-parse", sourceRef], sourceRoot).trim(),
    tree: runGit(["rev-parse", `${sourceRef}^{tree}`], sourceRoot).trim(),
  };
  if (expectedCommit && source.commit !== expectedCommit) {
    throw new Error(`source commit assertion mismatch: expected ${expectedCommit}, got ${source.commit}`);
  }
  if (expectedTree && source.tree !== expectedTree) {
    throw new Error(`source tree assertion mismatch: expected ${expectedTree}, got ${source.tree}`);
  }
  const dirtyPaths = parseStatus(runGit(["status", "--porcelain=v1", "--untracked-files=all", "-z"], sourceRoot));
  const unexpectedDirtyPaths = dirtyPaths.filter((path) => !OWNED_DIRTY_PATHS.has(path));
  if (unexpectedDirtyPaths.length > 0) {
    throw new Error(`inventory source has unexpected dirty paths: ${unexpectedDirtyPaths.sort().join(", ")}`);
  }
  const reviewedPredicates = DEFAULT_REVIEWED_PREDICATES;
  const semanticReviews = DEFAULT_SEMANTIC_DISPOSITIONS;
  const semanticReviewIndex = buildSemanticReviewIndex(semanticReviews);
  assertDispositionClaims(claimedDispositions, semanticReviews, source);

  const files = collectTrackedFiles({ sourceRoot,
    sourceRef: runGit === defaultRunGit ? source.commit : sourceRef, fsApi, runGit });
  const entries = [];
  const fragments = fragmentSourceViews(files);
  const locatedAnchors = new Set();
  const presentHistoricalPredicateIds = new Set();
  const appliedSemanticReviewIds = new Set();
  for (const file of fragments.files) {
    const rawLines = file.text.split(/\r?\n/);
    const views = file.format === "patch" ? patchSourceViews(file, (path) => classifyProductionPath(path, true))
      : [{ path: file.path, format: file.format, text: file.text, lineOffset: 0, patchLines: [] }];
    for (const view of views) {
      // Patch context may be omitted. Even with a verified fragment order,
      // the generic JavaScript mask cannot parse nested templates or regexes.
      // Retain raw fragment candidates, including apparent comments, for review.
      const preserveCandidates = file.format === "patch" || file.unverifiedFragment || file.assembly
        // MP-11: the generic mask does not parse XML or C++ raw literals.
        // Retain apparent comments in these new formats for semantic review.
        || [".plist", ".cc", ".h"].includes(extname(file.path).toLowerCase());
      const lines = (preserveCandidates ? view.text : stripComments(view.text, view.format)).split(/\r?\n/);
      const testRanges = view.format === "rust" ? findRustTestRanges(view.text, view.path) : [];
      const unitTopology = explicitUnitTopology(view.path, view.text);
      const embeddedPath = file.format === "patch" ? view.path : null;
      const manuals = sourceRuleCandidates(file, lines, embeddedPath, view.lineOffset);
      if (file.format !== "patch" && !file.assembly) {
        for (const current of currentDeclarationCandidates(file, source)) {
          if (!manuals.some((manual) => manual.ruleId === current.ruleId && manual.symbol === current.symbol
            && manual.lineIndex === current.lineIndex)) manuals.push(current);
        }
      }
      for (const manual of manuals) locatedAnchors.add(manual.ruleId + ":" + manual.symbol);
      for (let lineIndex = 0; lineIndex < lines.length; lineIndex += 1) {
        const scanLine = lines[lineIndex];
        if (!scanLine.trim() || (view.onlyRemoved && view.patchLines[lineIndex]?.change !== "removed")) continue;
        const lexical = CATEGORY_SPECS.flatMap((spec) => lineMatches(spec, scanLine).map((match) => ({
          category: spec.category, applicableMpIds: [...new Set(spec.mpIds)].sort(),
          affectedBehavior: spec.affectedBehavior, selector: redact(match.value),
          column: match.index + 1, matchLength: match.value.length, symbol: inferSymbol(lines, lineIndex), candidateOrigin: "lexical",
        })));
        const findings = [...lexical, ...manuals.filter((manual) => manual.lineIndex === lineIndex)
          .map((manual) => ({ ...manual, candidateOrigin: "manual_source_rule" }))];
        for (const match of findings) {
          const physicalLine = view.lineOffset + lineIndex;
          const anchor = file.assembly
            ? fragmentMatchAnchor(file, lineIndex, match.column, match.matchLength ?? match.selector.length)
            : { path: file.path, blob: file.blob, line: physicalLine + 1,
                column: match.column + (file.format === "patch" ? 1 : 0), sourceLine: rawLines[physicalLine].trim() };
          const sourceLine = anchor.sourceLine;
          const finding = {
            category: match.category, format: file.format, path: anchor.path, blob: anchor.blob,
            line: anchor.line, column: anchor.column,
            symbol: match.symbol, selector: match.selector, sourceLine,
            contextHash: sha256(sourceLine), affectedBehavior: match.affectedBehavior,
            applicableMpIds: match.applicableMpIds,
          };
          const sourceRoleHints = inferSourceRoleHints({
            path: view.path, format: view.format, lines, lineIndex,
            testRanges, unitTopology, category: match.category,
          });
          const patchSource = view.patchLines[lineIndex];
          if (patchSource) {
            sourceRoleHints.lexicalContext = "unknown_patch_fragment";
            sourceRoleHints.executionRole = "patch_fragment_candidate";
          }
          if (file.unverifiedFragment) {
            sourceRoleHints.lexicalContext = "unknown_fragment_assembly";
            sourceRoleHints.executionRole = "unverified_fragment_candidate";
          }
          if (file.assembly) {
            sourceRoleHints.lexicalContext = "unparsed_fragment_assembly";
            sourceRoleHints.executionRole = "fragment_candidate";
          }
          if (patchSource?.change === "removed" || view.oldOnly) {
            sourceRoleHints.executionRole = "removed_patch_source_candidate";
            sourceRoleHints.positivePath1Directive = false;
          }
          for (const predicate of reviewedPredicates) {
            if (historicalPredicateMatches(finding, predicate, source)) presentHistoricalPredicateIds.add(predicate.id);
          }
          delete finding.sourceLine;
          const candidateId = sha256(stableJson({ commit: source.commit, tree: source.tree, ...candidateAnchor(finding) }));
          const disposition = resolveSemanticDisposition(finding, source, semanticReviewIndex);
          if (disposition.status === "reviewed") appliedSemanticReviewIds.add(disposition.reviewId);
          entries.push({
            candidateId, ...finding, candidateOrigin: match.candidateOrigin, sourceRoleHints,
            ...(match.currentDeclarationExpectation ? { currentDeclarationExpectation: true } : {}),
            ...(patchSource ? { patchSource } : {}),
            ...(anchor.fragmentSource ? { fragmentSource: anchor.fragmentSource } : {}),
            sourceClassification: sourceClassification(file, embeddedPath, physicalLine + 1),
            semanticDisposition: disposition,
          });
        }
      }
    }
  }
  const auditGaps = [...fragments.gaps, ...sourceAuditGaps(files, locatedAnchors)];
  entries.sort((left, right) => JSON.stringify(left).localeCompare(JSON.stringify(right)));
  const observedCategories = [...new Set(entries.map((entry) => entry.category))].sort();
  const missingCategories = REQUIRED_CATEGORIES.filter((category) => !observedCategories.includes(category));
  const rowCoverage = buildRowCoverage(entries);
  const formats = Object.fromEntries([...new Set(files.map((file) => file.format))].sort().map((format) => [
    format,
    files.filter((file) => file.format === format).length,
  ]));
  const missingRows = rowCoverage.filter((row) => row.status === "missing").map((row) => row.id);
  const removalRequired = entries.filter((entry) => entry.semanticDisposition.disposition === "removal_required").length;
  const unreviewed = entries.filter((entry) => entry.semanticDisposition.status !== "reviewed").length;
  const reviewedPredicateStatus = reviewedPredicates.map((predicate) => {
    const sourceMatches = predicate.sourceCommit === source.commit && predicate.sourceTree === source.tree;
    const anchorPresent = presentHistoricalPredicateIds.has(predicate.id);
    return {
      id: predicate.id,
      sourceCommit: predicate.sourceCommit,
      sourceTree: predicate.sourceTree,
      status: !sourceMatches ? "pending_source_review"
        : anchorPresent ? "pending_independent_review" : "source_drift",
    };
  });
  const pendingReviewedPredicates = reviewedPredicateStatus.filter((predicate) => predicate.status !== "applied").length;
  const semanticReviewStatus = semanticReviews.map((review) => {
    const sourceMatches = review.sourceCommit === source.commit && review.sourceTree === source.tree;
    return {
      id: review.id,
      sourceCommit: review.sourceCommit,
      sourceTree: review.sourceTree,
      independentReviewer: review.independentReview.reviewer,
      independentReviewId: review.independentReview.reviewId,
      status: !sourceMatches ? "pending_source_review"
        : appliedSemanticReviewIds.has(review.independentReview.reviewId) ? "applied" : "source_drift",
    };
  });
  const pendingSemanticReviews = semanticReviewStatus.filter((review) => review.status !== "applied").length;
  // MP-11: retained historical/other-repository records stay visible, but
  // cannot be reapplied to this source. A missing same-source reviewed anchor
  // still blocks the gate, including approved anchors that disappeared.
  const unappliedCurrentSemanticReviews = semanticReviewStatus.filter((review) =>
    review.sourceCommit === source.commit && review.sourceTree === source.tree && review.status !== "applied").length;
  const report = {
    schema: INVENTORY_SCHEMA,
    source: {
      commit: source.commit,
      tree: source.tree,
      ref: source.ref,
      trackedFileCount: files.length,
      formats,
    },
    inventoryTool: inventoryToolIdentity(),
    reviewedPredicates: reviewedPredicateStatus,
    semanticReviews: semanticReviewStatus,
    rows: rowCoverage,
    requiredCategories: [...REQUIRED_CATEGORIES],
    observedCategories,
    missingCategories,
    missingRows,
    entries,
    sourceClassifications: groupSourceClassifications(entries),
    sourceAuditGaps: auditGaps,
    fragmentAssemblies: fragments.assemblies,
    summary: {
      candidateCount: entries.length,
      unresolvedSourceAuditGaps: auditGaps.length,
      removalRequired,
      unreviewed,
      pendingReviewedPredicates,
      pendingSemanticReviews,
      unappliedCurrentSemanticReviews,
      allowedReleaseDeployment: entries.filter((entry) => entry.semanticDisposition.disposition === "allowed_release_deployment").length,
      requiredAutomaticShutdown: entries.filter((entry) => entry.semanticDisposition.disposition === "required_automatic_shutdown").length,
    },
    status: auditGaps.length === 0 && missingCategories.length === 0
      && missingRows.length === 0
      && removalRequired === 0
      && unreviewed === 0
      && unappliedCurrentSemanticReviews === 0
      ? "pass"
      : "fail",
  };
  return stable(report);
}

export function parseArgs(argv) {
  const options = {
    root: resolve(dirname(fileURLToPath(import.meta.url)), "../../.."),
    output: null,
    expectedCommit: null,
    expectedTree: null,
    sourceRef: DEFAULT_SOURCE_REF,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--root") options.root = resolve(argv[++index]);
    else if (argument === "--output") options.output = resolve(argv[++index]);
    else if (argument === "--expect-source-commit") options.expectedCommit = argv[++index];
    else if (argument === "--expect-source-tree") options.expectedTree = argv[++index];
    else if (argument === "--source-ref") options.sourceRef = argv[++index];
    else if (argument === "--help") options.help = true;
    else throw new Error(`unknown argument: ${argument}`);
  }
  return options;
}

export function runCli(argv = process.argv.slice(2), io = { write: (value) => process.stdout.write(value), error: (value) => process.stderr.write(value) }) {
  const options = parseArgs(argv);
  if (options.help) {
    io.write("usage: node managed-parity-source-inventory.mjs [--root DIR] [--source-ref REF] [--expect-source-commit SHA] [--expect-source-tree TREE] [--output FILE]\n");
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
