import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import {
  collectSourceInventory,
  DEFAULT_SOURCE_REF,
  MP_ROWS,
  parseArgs,
  parseCatFileBatch,
  parseStatus,
  PRIOR_REVIEWED_SOURCE_COMMIT,
  PRIOR_REVIEWED_SOURCE_TREE,
  stableJson,
} from "./managed-parity-source-inventory.mjs";

const COMMIT = PRIOR_REVIEWED_SOURCE_COMMIT;
const TREE = PRIOR_REVIEWED_SOURCE_TREE;

function fixtureFiles({ hiddenManagedFlag = false, directBwrap = false, inheritedRestriction = false, unknownProjection = false, protectedParent = true, errorMapping = true, shutdown = true } = {}) {
  const release = [...Array(38).fill("// reviewed release fixture"), "pub(super) fn verify_release(", "  manifest_path: &Path,", ");"].join("\n") + "\n";
  const autoStop = [
    ...Array(108).fill("// reviewed auto-stop fixture"),
    "struct AutoStopPolicy {",
    "    minimum_runtime_seconds: u64,",
    "    idle_delay_seconds: Option<u64>,",
    "}",
  ].join("\n") + "\n";
  const files = {
    "apps/kernel/src/managed_bootstrap/release.rs": release,
    "apps/kernel/src/runtime/managed_environment_control/cloud_contract.rs": shutdown ? autoStop : "struct AutoStopPolicy {\n    policy_name: String,\n}\n",
    "apps/kernel/src/selector-rust.rs": [
      "// CHARIOX_MANAGED_COMMENT_ONLY_RUST",
      "let selector = std::env::var_os(\"CHARIOX_MANAGED_RUST_SELECTOR\");",
    ].join("\n") + "\n",
    "apps/kernel/src/ordinary-env.rs": "let endpoint = std::env::var(\"CHARIOX_PUBLICATION_CLOUD_API_URL\");\n",
    "apps/kernel/src/comments.rs": [
      "// CHARIOX_MANAGED_LINE_COMMENT_ONLY",
      "/* CHARIOX_PUBLICATION_CONTROL_STATE_DIR_BLOCK_COMMENT_ONLY */",
    ].join("\n") + "\n",
    "apps/kernel/src/provider/managed_isolation.rs": [
      "fn managed_mode_fixture() {",
      hiddenManagedFlag
        ? "  let hidden = std::env::var(\"CHARIOX_MANAGED_HIDDEN_FLAG\");"
        : "  let ordinary = \"ordinary reviewed selector fixture\";",
      directBwrap ? "  let command = \"/usr/bin/bwrap\";" : "  let command = \"ordinary-provider\";",
      "  if (is_managed) { run_managed(); }",
      "}",
    ].join("\n") + "\n",
    "apps/cli/src/selector-typescript.ts": [
      "// CHARIOX_DISPOSABLE_WORKER_RECEIPT_COMMENT_ONLY",
      "const selector = process.env.CHARIOX_DISPOSABLE_WORKER_RECEIPT;",
    ].join("\n") + "\n",
    "apps/cli/scripts/live-managed-selector.sh": "receipt=\"${CHARIOX_MANAGED_CLI_SCRIPT_SELECTOR:?}\"\n",
    "apps/cli/scripts/managed-parity-source-inventory.mjs": "const selfScan = CHARIOX_MANAGED_SELF_SCAN_FALSE_POSITIVE;\n",
    "apps/ios/CharioxPackage/Sources/Selector.swift": [
      "// CHARIOX_MANAGED_SWIFT_COMMENT_ONLY",
      "let selector = ProcessInfo.processInfo.environment[\"CHARIOX_PUBLICATION_CONTROL_STATE_DIR\"]",
    ].join("\n") + "\n",
    "scripts/selector.sh": [
      "# CHARIOX_MANAGED_SHELL_COMMENT_ONLY",
      "receipt=\"${CHARIOX_DISPOSABLE_WORKER_RECEIPT:?}\"",
    ].join("\n") + "\n",
    "deploy/managed-kernel/provider.service": [
      inheritedRestriction ? "RestrictAddressFamilies=AF_UNIX" : "NoNewPrivileges=true",
      "Environment=CHARIOX_PUBLICATION_CONTROL_STATE_DIR=/var/lib/chariox/publication",
      "ProtectSystem=strict",
    ].join("\n") + "\n",
    "docker/selector-image/Dockerfile": [
      "# CHARIOX_MANAGED_DOCKER_COMMENT_ONLY",
      "ENV CHARIOX_MANAGED_DOCKER_SELECTOR=1",
    ].join("\n") + "\n",
    "apps/kernel/slice-linux-docker/selector.apparmor": [
      "# CHARIOX_MANAGED_APPARMOR_COMMENT_ONLY",
      "profile chariox-selector {",
      "  /usr/bin/env CHARIOX_MANAGED_APPARMOR_SELECTOR rix,",
      "}",
    ].join("\n") + "\n",
    "apps/kernel/slice-linux-docker/docker/managed-provider-seccomp.c": [
      "// CHARIOX_MANAGED_C_COMMENT_ONLY",
      "const char *selector = \"CHARIOX_MANAGED_C_SELECTOR\";",
    ].join("\n") + "\n",
    "apps/kernel/slice-linux-docker/docker/slice-selkies.py": [
      "# CHARIOX_MANAGED_PYTHON_COMMENT_ONLY",
      "selector = os.environ[\"CHARIOX_MANAGED_PYTHON_SELECTOR\"]",
    ].join("\n") + "\n",
    "apps/kernel/src/managed_context/protected.rs": protectedParent ? "fn protected_parent_filter() { let protected_path = true; }\n" : "fn ordinary_path_filter() {}\n",
    "apps/kernel/src/generated/false.rs": "const GENERATED = CHARIOX_MANAGED_GENERATED_FALSE_POSITIVE;\n",
    "apps/kernel/src/tests/false.rs": "const TEST_ONLY = CHARIOX_PUBLICATION_CONTROL_STATE_DIR_TEST_FALSE_POSITIVE;\n",
    "apps/kernel/src/runtime/selector.test.rs": "const TEST_SUFFIX = CHARIOX_MANAGED_TEST_SUFFIX_FALSE_POSITIVE;\n",
    "packages/aegs-sdk/src/conformance_attestation.rs": "let selector = std::env::var(\"CHARIOX_MANAGED_ATTESTATION_SELECTOR\");\n",
    "apps/kernel/src/error_map.rs": errorMapping ? "let managed =\u0085 failure; const token = \"secret-value\";\n" : "fn ordinary_error() {}\n",
    "apps/kernel/src/path.rs": "const ROOT = CHARIOX_MANAGED_REPOSITORY_ROOT; // /home/chariox and /tmp\n",
    "apps/kernel/src/cleanup.rs": "let managed = cleanup;\n",
    "apps/cli/src/client.ts": "const managed = projection;\n",
    "apps/kernel/slice-linux-docker/docker/Dockerfile": "RUN bwrap --unshare-user --die-with-parent\n",
  };
  if (unknownProjection) files["apps/client/unknown.ts"] = "const value = { managed: projection };\n";
  return files;
}

function makeFixture(options = {}) {
  const root = mkdtempSync(join(tmpdir(), "chariox-mp11-"));
  const files = fixtureFiles(options);
  const entries = [];
  const blobByPath = new Map();
  const blobsById = new Map();
  let index = 1;
  const addFile = (file, contents, mode = "100644") => {
    const absolute = join(root, file);
    mkdirSync(dirname(absolute), { recursive: true });
    writeFileSync(absolute, contents);
    const blob = String(index).padStart(40, "0");
    entries.push({ mode, blob, file });
    blobByPath.set(file, blob);
    blobsById.set(blob, Buffer.from(contents));
    files[file] = contents;
    index += 1;
  };
  const replaceFile = (file, contents) => {
    const prior = entries.find((entry) => entry.file === file);
    if (!prior) throw new Error(`fixture file not found: ${file}`);
    entries.splice(entries.indexOf(prior), 1);
    addFile(file, contents, prior.mode);
  };
  for (const [file, contents] of Object.entries(files)) {
    addFile(file, contents);
  }
  const runGit = (args) => {
    if (args[0] === "rev-parse" && args[1] === "HEAD") return `${COMMIT}\n`;
    if (args[0] === "rev-parse" && args[1].endsWith("^{tree}")) return `${TREE}\n`;
    if (args[0] === "status") return options.dirtyPath ? ` M ${options.dirtyPath}\0` : "";
    if (args[0] === "ls-tree") {
      const records = [...entries].sort((left, right) => left.file.localeCompare(right.file))
        .map(({ mode, blob, file }) => `${mode} blob ${blob}\t${file}`);
      return `${records.join("\0")}\0`;
    }
    throw new Error(`unexpected git command in fixture: ${args.join(" ")}`);
  };
  const readBlobBatch = (_sourceRoot, blobIds) => new Map(blobIds.map((blob) => [blob, blobsById.get(blob)]));
  return { root, files, addFile, replaceFile, blobByPath, readBlobBatch, runGit };
}

test("porcelain NUL records retain both rename/copy paths and literal arrows", () => {
  assert.deepEqual(parseStatus("R  new name\0old name\0 C copy\0original\0 M literal -> name\0"),
    ["new name", "old name", "copy", "original", "literal -> name"]);
  assert.throws(() => parseStatus("R  new\0"), /incomplete git status/);
});

test("cat-file batch parsing rejects absent, mismatched, malformed, and truncated objects", () => {
  const blob = "a".repeat(40);
  const otherBlob = "b".repeat(40);
  assert.deepEqual(parseCatFileBatch(Buffer.from(`${blob} blob 3\nabc\n`), [blob]).get(blob), Buffer.from("abc"));
  assert.throws(() => parseCatFileBatch(Buffer.from(`${blob} missing\n`), [blob]), /could not find listed blob/);
  assert.throws(() => parseCatFileBatch(Buffer.from(`${otherBlob} blob 1\nx\n`), [blob]), /different object than requested/);
  assert.throws(() => parseCatFileBatch(Buffer.from(`${blob} tree 1\nx\n`), [blob]), /malformed object header/);
  assert.throws(() => parseCatFileBatch(Buffer.from(`${blob} blob 4\nabc`), [blob]), /truncated or malformed/);
  assert.throws(() => parseCatFileBatch(Buffer.from(`${blob} blob 0\n\nextra`), [blob]), /unexpected trailing bytes/);
});

test("assume-unchanged worktree edits do not replace committed source bytes", () => {
  const root = mkdtempSync(join(tmpdir(), "chariox-mp11-committed-blobs-"));
  const sourcePath = "apps/kernel/src/identity.rs";
  const oldSource = 'let selector = "CHARIOX_MANAGED_OLD_REF_SOURCE";\n';
  const committedSource = 'let selector = "CHARIOX_MANAGED_COMMITTED_SOURCE";\n';
  const modifiedSource = 'let selector = "CHARIOX_MANAGED_WORKTREE_DECOY";\n';
  const gitEnv = { ...process.env, GIT_CONFIG_GLOBAL: "/dev/null", GIT_CONFIG_NOSYSTEM: "1" };
  delete gitEnv.GIT_NO_REPLACE_OBJECTS;
  const git = (...args) => execFileSync("git", args, {
    cwd: root, encoding: "utf8",
    env: gitEnv,
  });
  try {
    mkdirSync(dirname(join(root, sourcePath)), { recursive: true });
    writeFileSync(join(root, sourcePath), oldSource);
    git("init", "--quiet");
    git("config", "user.name", "Inventory Test");
    git("config", "user.email", "inventory-test@example.invalid");
    git("add", sourcePath);
    git("commit", "--quiet", "-m", "old ref fixture");
    const oldCommit = git("rev-parse", "HEAD").trim();
    writeFileSync(join(root, sourcePath), committedSource);
    git("add", sourcePath);
    git("commit", "--quiet", "-m", "committed source fixture");
    git("update-index", "--assume-unchanged", sourcePath);
    writeFileSync(join(root, sourcePath), modifiedSource);
    assert.equal(git("status", "--porcelain=v1", "--untracked-files=all").trim(), "");
    const headReport = collectSourceInventory({ sourceRoot: root });
    assert.ok(headReport.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_COMMITTED_SOURCE"));
    assert.equal(headReport.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_WORKTREE_DECOY"), false);
    const oldRefReport = collectSourceInventory({ sourceRoot: root, sourceRef: oldCommit });
    assert.ok(oldRefReport.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_OLD_REF_SOURCE"));
    assert.equal(oldRefReport.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_WORKTREE_DECOY"), false);
    const committedBlob = git("rev-parse", `HEAD:${sourcePath}`).trim();
    const decoyBlob = git("hash-object", "-w", sourcePath).trim();
    git("replace", committedBlob, decoyBlob);
    // Prove the replacement is active in the fixture, then ensure inventory ignores it.
    assert.equal(git("cat-file", "blob", committedBlob), modifiedSource);
    const replacedReport = collectSourceInventory({ sourceRoot: root });
    assert.ok(replacedReport.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_COMMITTED_SOURCE"));
    assert.equal(replacedReport.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_WORKTREE_DECOY"), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("non-HEAD source blobs use the same binary exclusion as HEAD", () => {
  withFixture({}, (fixture) => {
    const runGit = (args, cwd) => {
      if (args[0] === "rev-parse") return args[1].endsWith("^{tree}") ? `${TREE}\n` : `${COMMIT}\n`;
      return fixture.runGit(args, cwd);
    };
    const readBlobBatch = (sourceRoot, blobIds) => {
      const blobs = fixture.readBlobBatch(sourceRoot, blobIds);
      blobs.set(fixture.blobByPath.get("apps/kernel/src/selector-rust.rs"), Buffer.from("\0CHARIOX_MANAGED_BINARY"));
      return blobs;
    };
    const report = collect(fixture, { sourceRef: "snapshot", readBlobBatch, runGit });
    assert.equal(report.entries.some(entry => entry.path === "apps/kernel/src/selector-rust.rs"), false);
  });
});

test("lifetimes and regex quotes cannot hide executable selectors after URLs", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("apps/kernel/src/lifetime.rs", `const NAME: &'static str = "x";\nconst ROOT: &str = "it's";\nlet u = "https://h/p"; std::env::var("CHARIOX_MANAGED_LIFETIME");\n`);
    fixture.addFile("apps/cli/src/regexp.ts", `const q = /["]/;\nconst u = "http://x"; process.env.CHARIOX_MANAGED_REGEXP;\n`);
    const report = collect(fixture);
    for (const selector of ["CHARIOX_MANAGED_LIFETIME", "CHARIOX_MANAGED_REGEXP"]) {
      assert.ok(report.entries.some(entry => entry.selector === selector), selector);
    }
  });
});

test("code between block comments remains visible", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("apps/kernel/src/comments.rs",
      '/* a */ let x = std::env::var("CHARIOX_MANAGED_BETWEEN"); /* b */\n/* CHARIOX_MANAGED_COMMENT_ONLY */\n');
    const report = collect(fixture);
    assert.ok(report.entries.some(entry => entry.selector === "CHARIOX_MANAGED_BETWEEN"));
    assert.equal(report.entries.some(entry => entry.selector === "CHARIOX_MANAGED_COMMENT_ONLY"), false);
  });
});

test("MP row meanings stay aligned with the stable plan ledger", () => {
  assert.deepEqual(MP_ROWS.map(({ id, name }) => [id, name]), [
    ["MP-01", "remove Path-1 Bubblewrap and inherited managed sandboxing"],
    ["MP-02", "ordinary directory discovery, exact-path entry, and provider access"],
    ["MP-03", "protect exact managed control state without blocking siblings"],
    ["MP-04", "ordinary HOME and CHARIOX_HOME state layout"],
    ["MP-05", "source-basename repository materialization and collision safety"],
    ["MP-06", "server-authoritative custom repository root"],
    ["MP-07", "signed content-addressed release activation"],
    ["MP-08", "ordinary kernel runtime, protocol, adapters, state, and clients"],
    ["MP-09", "mandatory managed automatic-shutdown lifecycle"],
    ["MP-10", "fresh-machine ordinary-versus-managed comparison and cleanup evidence"],
    ["MP-11", "proactive managed-only source inventory"],
  ]);
});

function collect(fixture, options = {}) {
  return collectSourceInventory({
    sourceRoot: fixture.root,
    readBlobBatch: fixture.readBlobBatch,
    runGit: fixture.runGit,
    expectedCommit: COMMIT,
    expectedTree: TREE,
    ...options,
  });
}

function withFixture(options, callback) {
  const fixture = makeFixture(options);
  try {
    return callback(fixture);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
}

test("exactly pinned source fixture applies only path, line, symbol, and text matches", () => {
  withFixture({}, (fixture) => {
    const report = collect(fixture);
    assert.equal(report.source.commit, COMMIT);
    assert.equal(report.source.tree, TREE);
    assert.deepEqual(report.missingCategories, []);
    assert.deepEqual(report.missingRows, []);
    assert.equal(report.summary.allowedReleaseDeployment, 1);
    assert.equal(report.summary.requiredAutomaticShutdown, 2);
    assert.equal(report.summary.pendingReviewedPredicates, 0);
    assert.deepEqual(report.reviewedPredicates.map(({ status }) => status), ["applied", "applied", "applied"]);
    assert.equal(report.status, "fail", "unapproved managed selectors must fail closed");
    assert.ok(report.entries.some((entry) => entry.category === "managed_env_selector"));
    assert.ok(report.entries.some((entry) => entry.category === "cleanup_selector"));
  });
});

test("production scripts are scanned while this inventory tool is excluded", () => {
  withFixture({}, (fixture) => {
    const report = collect(fixture);
    assert.ok(report.entries.some((entry) => entry.path === "apps/cli/scripts/live-managed-selector.sh"
      && entry.selector === "CHARIOX_MANAGED_CLI_SCRIPT_SELECTOR"));
    assert.equal(report.entries.some((entry) => entry.path === "apps/cli/scripts/managed-parity-source-inventory.mjs"), false);
  });
});

test("default CLI follows current HEAD without treating it as reviewed", () => {
  const options = parseArgs([]);
  assert.equal(options.sourceRef, DEFAULT_SOURCE_REF);
  assert.equal(options.sourceRef, "HEAD");
  assert.equal(options.expectedCommit, null);
  assert.equal(options.expectedTree, null);
  assert.deepEqual(parseArgs(["--expect-source-commit", COMMIT, "--expect-source-tree", TREE]), {
    ...options,
    expectedCommit: COMMIT,
    expectedTree: TREE,
  });
});

test("dirty production source cannot be reported under the committed tree identity", () => {
  withFixture({ dirtyPath: "apps/cli/package.json" }, (fixture) => {
    assert.throws(() => collect(fixture), /unexpected dirty paths: apps\/cli\/package\.json/);
  });
});

test("historical approvals stay pending on current source instead of being repinned", () => {
  withFixture({}, (fixture) => {
    const currentCommit = "a".repeat(40);
    const currentTree = "b".repeat(40);
    const runGit = (args, cwd) => {
      if (args[0] === "rev-parse" && args[1] === "HEAD") return `${currentCommit}\n`;
      if (args[0] === "rev-parse" && args[1].endsWith("^{tree}")) return `${currentTree}\n`;
      return fixture.runGit(args, cwd);
    };
    const report = collect(fixture, {
      runGit,
      expectedCommit: currentCommit,
      expectedTree: currentTree,
    });
    assert.equal(report.source.commit, currentCommit);
    assert.equal(report.source.tree, currentTree);
    assert.equal(report.summary.allowedReleaseDeployment, 0);
    assert.equal(report.summary.requiredAutomaticShutdown, 0);
    assert.equal(report.summary.pendingReviewedPredicates, 3);
    assert.ok(report.reviewedPredicates.every(({ status }) => status === "pending_source_review"));
    assert.equal(report.status, "fail");
  });
});

test("all supported production formats and managed selector families are inventoried", () => {
  withFixture({}, (fixture) => {
    const report = collect(fixture);
    const expectedFormats = new Map([
      ["apps/kernel/src/selector-rust.rs", "rust"],
      ["apps/cli/src/selector-typescript.ts", "javascript"],
      ["apps/ios/CharioxPackage/Sources/Selector.swift", "swift"],
      ["scripts/selector.sh", "shell"],
      ["deploy/managed-kernel/provider.service", "unit"],
      ["docker/selector-image/Dockerfile", "container"],
      ["apps/kernel/slice-linux-docker/selector.apparmor", "policy"],
      ["apps/kernel/slice-linux-docker/docker/managed-provider-seccomp.c", "c"],
      ["apps/kernel/slice-linux-docker/docker/slice-selkies.py", "python"],
      ["packages/aegs-sdk/src/conformance_attestation.rs", "rust"],
    ]);
    for (const [path, format] of expectedFormats) {
      assert.ok(report.entries.some((entry) => entry.path === path && entry.format === format), `${path} should be ${format}`);
    }
    assert.ok(report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_RUST_SELECTOR"));
    assert.ok(report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_C_SELECTOR"));
    assert.ok(report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_PYTHON_SELECTOR"));
    assert.ok(report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_ATTESTATION_SELECTOR"));
    assert.ok(report.entries.some((entry) => entry.selector === "CHARIOX_PUBLICATION_CONTROL_STATE_DIR"));
    assert.ok(report.entries.some((entry) => entry.selector === "CHARIOX_DISPOSABLE_WORKER_RECEIPT"));
    assert.equal(report.entries.some((entry) => entry.selector === "CHARIOX_PUBLICATION_CLOUD_API_URL"), false);
    assert.equal(report.entries.some((entry) => entry.selector.includes("COMMENT_ONLY")), false);
    assert.equal(report.entries.some((entry) => entry.path.includes("/generated/")), false);
    assert.equal(report.entries.some((entry) => entry.path.includes("/tests/")), false);
    assert.equal(report.entries.some((entry) => entry.path.endsWith("selector.test.rs")), false);
    assert.ok(report.source.formats.rust >= 1);
    assert.ok(report.source.formats.javascript >= 1);
    assert.ok(report.source.formats.swift >= 1);
    assert.ok(report.source.formats.shell >= 1);
    assert.ok(report.source.formats.unit >= 1);
    assert.ok(report.source.formats.container >= 1);
    assert.ok(report.source.formats.policy >= 1);
    assert.ok(report.source.formats.c >= 1);
    assert.ok(report.source.formats.python >= 1);
  });
});

test("redaction preserves lowercase executable selectors while removing controls", () => {
  withFixture({}, (fixture) => {
    const report = collect(fixture);
    assert.ok(report.entries.some((entry) => entry.selector === "bwrap"));
    assert.ok(report.entries.some((entry) => entry.selector === "protected_path"));
    const controlSafe = report.entries.find((entry) => entry.category === "managed_only_error_mapping");
    assert.match(controlSafe?.selector ?? "", /managed = failure/);
    assert.doesNotMatch(controlSafe?.selector ?? "", /[\u0000-\u001f]/u);
    assert.doesNotMatch(controlSafe?.selector ?? "", /[\u007f-\u009f]/u);
    assert.equal(report.entries.some((entry) => entry.selector === "" || entry.selector === "_"), false);
  });
});

test("an eligible production file with an unknown format fails closed", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("apps/kernel/src/unsupported.selector", "CHARIOX_MANAGED_UNCLASSIFIED_SELECTOR\n");
    assert.throws(() => collect(fixture), /unclassified production file: apps\/kernel\/src\/unsupported\.selector/);
  });
});

test("tracked repository ignore dotfiles are excluded as bounded metadata", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("apps/ios/.gitignore", "DerivedData/\n");
    fixture.addFile("docker/.dockerignore", "build/\n");
    fixture.addFile("deploy/.env", "CHARIOX_MANAGED_DOTENV=1\n");
    fixture.addFile("apps/kernel/.charioxignore", "");
    fixture.addFile("apps/kernel/slice-linux-docker/prebuilt/.gitkeep", "\n");
    const report = collect(fixture);
    assert.equal(report.entries.some((entry) => entry.path === "apps/ios/.gitignore"), false);
    assert.equal(report.entries.some((entry) => entry.path === "docker/.dockerignore"), false);
    assert.ok(report.entries.some((entry) => entry.path === "deploy/.env"));
    assert.equal(report.entries.some((entry) => entry.path === "apps/kernel/.charioxignore"), false);
    assert.equal(report.entries.some((entry) => entry.path === "apps/kernel/slice-linux-docker/prebuilt/.gitkeep"), false);
  });
});

test("a new hidden CHARIOX_MANAGED flag is a direct Path-1 removal finding", () => {
  withFixture({ hiddenManagedFlag: true }, (fixture) => {
    const report = collect(fixture);
    const hidden = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_HIDDEN_FLAG");
    assert.equal(hidden?.topology, "direct_path1");
    assert.equal(hidden?.disposition, "removal_required");
  });
});

test("Bubblewrap added to direct Path 1 is not confused with the Docker slice", () => {
  withFixture({ directBwrap: true }, (fixture) => {
    const report = collect(fixture);
    const direct = report.entries.find((entry) => entry.category === "bubblewrap" && entry.path.includes("managed_isolation"));
    assert.equal(direct?.topology, "direct_path1");
    assert.equal(direct?.disposition, "removal_required");
    const docker = report.entries.find((entry) => entry.category === "bubblewrap" && entry.topology === "inner_docker_slice");
    assert.equal(docker?.disposition, "unreviewed");
    assert.equal(docker?.path, "apps/kernel/slice-linux-docker/docker/Dockerfile");
  });
});

test("an inherited systemd restriction remains a direct Path-1 removal finding", () => {
  withFixture({ inheritedRestriction: true }, (fixture) => {
    const report = collect(fixture);
    const restriction = report.entries.find((entry) => entry.selector === "RestrictAddressFamilies");
    assert.equal(restriction?.category, "managed_service_restriction");
    assert.equal(restriction?.topology, "direct_path1");
    assert.equal(restriction?.disposition, "removal_required");
  });
});

test("protected-parent filtering is inventoried as MP-02/MP-03/MP-11 evidence", () => {
  withFixture({ protectedParent: true }, (fixture) => {
    const report = collect(fixture);
    const protectedFinding = report.entries.find((entry) => entry.category === "protected_path_filter");
    assert.deepEqual(protectedFinding?.applicableMpIds, ["MP-02", "MP-03", "MP-11"]);
    assert.equal(protectedFinding?.disposition, "removal_required");
  });
});

test("an unknown client projection is unreviewed rather than allowed by its filename", () => {
  withFixture({ unknownProjection: true }, (fixture) => {
    const report = collect(fixture);
    const projection = report.entries.find((entry) => entry.path === "apps/client/unknown.ts");
    assert.equal(projection?.category, "client_projection");
    assert.equal(projection?.topology, "unknown");
    assert.equal(projection?.disposition, "unreviewed");
  });
});

test("a caller cannot forge an allowed disposition", () => {
  withFixture({}, (fixture) => {
    assert.throws(() => collect(fixture, {
      claimedDispositions: [{
        path: "apps/kernel/src/provider/evil.rs",
        line: 1,
        symbol: "evil",
        category: "managed_env_selector",
        sourceLine: "CHARIOX_MANAGED_FORGED",
        disposition: "allowed_release_deployment",
      }],
    }), /unverified disposition claim/);
  });
});

test("source drift removes the exact release exemption", () => {
  withFixture({}, (fixture) => {
    const releasePath = join(fixture.root, "apps/kernel/src/managed_bootstrap/release.rs");
    const original = readFileSync(releasePath, "utf8");
    fixture.replaceFile("apps/kernel/src/managed_bootstrap/release.rs", original.replace("pub(super) fn verify_release(", "pub(super) fn verify_release( // drift"));
    const driftedCommit = "c".repeat(40);
    const driftedTree = "d".repeat(40);
    const runGit = (args, cwd) => {
      if (args[0] === "rev-parse" && args[1] === "HEAD") return `${driftedCommit}\n`;
      if (args[0] === "rev-parse" && args[1].endsWith("^{tree}")) return `${driftedTree}\n`;
      return fixture.runGit(args, cwd);
    };
    const report = collect(fixture, { runGit, expectedCommit: driftedCommit, expectedTree: driftedTree });
    const drifted = report.entries.find((entry) => entry.path.endsWith("release.rs") && entry.line === 39);
    assert.equal(drifted?.disposition, "unreviewed");
    assert.equal(report.summary.allowedReleaseDeployment, 0);
  });
});

test("a missing shutdown trigger fails MP-09 instead of being silently omitted", () => {
  withFixture({ shutdown: false }, (fixture) => {
    const report = collect(fixture);
    assert.ok(report.missingCategories.includes("automatic_shutdown_selector"));
    assert.ok(report.missingRows.includes("MP-09"));
    assert.equal(report.status, "fail");
  });
});

test("output is deterministic and redacts selector secrets", () => {
  withFixture({}, (fixture) => {
    const first = collect(fixture);
    const second = collect(fixture);
    assert.equal(stableJson(first), stableJson(second));
    const serialized = stableJson(first);
    assert.doesNotMatch(serialized, /secret-value/);
    assert.doesNotMatch(serialized, /sourceLine/);
    assert.match(serialized, /contextHash/);
  });
});
