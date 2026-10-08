import { runInNewContext } from "node:vm";
import assert from "node:assert/strict";
import { copyFileSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { INDEPENDENT_REVIEW_GROUPS } from "./lib/managed-parity-semantic-reviews.mjs";
import { CURRENT_DECLARATION_EXPECTATIONS, currentDeclarationCandidates } from "./lib/managed-parity-current-declarations.mjs";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { CURRENT_REVIEW_GROUPS, CURRENT_SEMANTIC_REVIEWS } from "./lib/managed-parity-current-reviews.mjs";
import { sourceRuleCandidates, SOURCE_AUDIT_RULES } from "./lib/managed-parity-source-rules.mjs";
import {
  collectSourceInventory,
  classifyProductionPath,
  DEFAULT_SOURCE_REF,
  DEFAULT_SEMANTIC_DISPOSITIONS,
  DEFAULT_REVIEWED_PREDICATES,
  evaluateSemanticDisposition,
  INVENTORY_SCHEMA,
  MP_ROWS,
  parseArgs,
  parseStatus,
  PRIOR_REVIEWED_SOURCE_COMMIT,
  PRIOR_REVIEWED_SOURCE_TREE,
  stableJson,
} from "./managed-parity-source-inventory.mjs";

const COMMIT = PRIOR_REVIEWED_SOURCE_COMMIT;
const TREE = PRIOR_REVIEWED_SOURCE_TREE;

test("MP-11 porcelain NUL records retain both rename/copy paths and literal arrows", () => {
  assert.deepEqual(parseStatus("R  new name\0old name\0 C copy\0original\0 M literal -> name\0"),
    ["new name", "old name", "copy", "original", "literal -> name"]);
  assert.throws(() => parseStatus("R  new\0"), /incomplete git status/);
});

test("MP-11 real Git status preserves rename/copy paths, spaces, Unicode and literal arrows", () => {
  const root = mkdtempSync(join(tmpdir(), "chariox-mp11-status-"));
  const git = (...args) => execFileSync("git", ["-c", "core.hooksPath=/dev/null", ...args], { cwd: root, encoding: "utf8" });
  try {
    git("init", "--quiet");
    writeFileSync(join(root, "old name"), "rename content\n".repeat(20));
    writeFileSync(join(root, "original ü"), "copy content\n".repeat(20));
    writeFileSync(join(root, "literal -> name"), "literal\n");
    git("add", ".");
    git("-c", "user.name=MP-11 fixture", "-c", "user.email=fixture@example.invalid",
      "-c", "commit.gpgSign=false", "commit", "--quiet", "-m", "MP-11 status fixture");
    git("mv", "old name", "new name");
    copyFileSync(join(root, "original ü"), join(root, "copy -> café"));
    // Git detects copies from modified sources with status.renames=copies.
    writeFileSync(join(root, "original ü"), "copy content\n".repeat(20) + "changed\n");
    writeFileSync(join(root, "literal -> name"), "literal modified\n");
    git("add", ".");
    writeFileSync(join(root, "untracked -> 日本語 name"), "untracked\n");
    const output = git("-c", "status.renames=copies", "status", "--porcelain=v1", "--untracked-files=all", "-z");
    assert.ok(output.includes("C  copy -> café\0original ü\0"), "real Git emitted a copy record");
    assert.ok(output.includes("R  new name\0old name\0"), "real Git emitted a rename record");
    assert.deepEqual(parseStatus(output), [
      "copy -> café", "original ü", "literal -> name", "new name", "old name", "original ü", "untracked -> 日本語 name",
    ]);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

function fixtureFiles({ hiddenManagedFlag = false, directBwrap = false, inheritedRestriction = false, unknownProjection = false, protectedParent = true, errorMapping = true, shutdown = true, path1ServiceTopology = "path1" } = {}) {
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
    "apps/kernel/src/managed_bootstrap/mod.rs": [
      "const PATH1_KERNEL_SLICE_BROKER_ENVS: &[&str] = &[",
      "    \"CHARIOX_SLICE_ROOT\",",
      "    \"CHARIOX_SLICE_DOCKER_BROKER_SOCKET\",",
      "    \"CHARIOX_SLICE_DOCKER_BROKER_FD\",",
      "    \"CHARIOX_SLICE_DOCKER_BROKER_REQUIRED\",",
      "];",
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
      "#[cfg(test)]",
      "mod tests {",
      "  #[test]",
      "  fn keeps_test_literals_visible_to_inventory() {",
      "    let fixture = \"bwrap CHARIOX_MANAGED_TEST_LITERAL\";",
      "    assert!(!fixture.is_empty());",
      "  }",
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
      ...(path1ServiceTopology ? [`Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=${path1ServiceTopology}`] : []),
      inheritedRestriction ? "RestrictAddressFamilies=AF_UNIX" : "NoNewPrivileges=true",
      "Environment=CHARIOX_PUBLICATION_CONTROL_STATE_DIR=/var/lib/chariox/publication",
      "ProtectSystem=strict",
      directBwrap ? "ExecStart=/usr/bin/bwrap --unshare-user chariox-managed-bootstrap" : "ExecStart=/usr/local/bin/chariox-managed-bootstrap",
    ].join("\n") + "\n",
    "deploy/managed-kernel/chariox-managed-bootstrap.service": [
      "Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=shared_host",
      "Environment=CHARIOX_MANAGED_PROVIDER_ISOLATION=1",
      "ProtectSystem=strict",
    ].join("\n") + "\n",
    "deploy/managed-kernel/verify-image-release.mjs": [
      "function verifyPath1Service(service) {",
      "  const forbidden = [\"bwrap\", \"ProtectSystem=\"];",
      "  for (const marker of forbidden) if (service.includes(marker)) fail(`forbidden ${marker}`);",
      "}",
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
    "apps/kernel/slice-linux-docker/docker/Dockerfile": [
      "FROM scratch AS managed-release-artifacts",
      "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-kernel /chariox-kernel",
      "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-managed-bootstrap /chariox-managed-bootstrap",
      "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-relay /chariox-relay",
      "RUN bwrap --unshare-user --die-with-parent",
    ].join("\n") + "\n",
  };
  if (unknownProjection) files["apps/client/unknown.ts"] = "const value = { managed: projection };\n";
  return files;
}

function makeFixture(options = {}) {
  const root = mkdtempSync(join(tmpdir(), "chariox-mp11-"));
  const files = fixtureFiles(options);
  const entries = [];
  let index = 1;
  const addFile = (file, contents, mode = "100644", blob = null) => {
    const absolute = join(root, file);
    mkdirSync(dirname(absolute), { recursive: true });
    writeFileSync(absolute, contents);
    const entry = `${mode} blob ${(blob ?? String(index).padStart(40, "0"))}\t${file}`;
    const existingIndex = entries.findIndex((tracked) => tracked.endsWith(`\t${file}`));
    if (existingIndex === -1) entries.push(entry);
    else entries[existingIndex] = entry;
    index += 1;
  };
  for (const [file, contents] of Object.entries(files)) {
    addFile(file, contents);
  }
  const runGit = (args) => {
    if (args[0] === "rev-parse" && args[1] === "HEAD") return `${COMMIT}\n`;
    if (args[0] === "rev-parse" && args[1] === "HEAD^{tree}") return `${TREE}\n`;
    if (args[0] === "status") return options.dirtyPath ? ` M ${options.dirtyPath}\0` : "";
    if (args[0] === "ls-tree") return `${entries.join("\0")}\0`;
    throw new Error(`unexpected git command in fixture: ${args.join(" ")}`);
  };
  return { root, files, addFile, runGit };
}

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

test("exactly pinned source fixture retains historical anchors without semantic approval", () => {
  withFixture({}, (fixture) => {
    const report = collect(fixture);
    assert.equal(report.schema, INVENTORY_SCHEMA);
    assert.equal(INVENTORY_SCHEMA, "chariox.managed-parity.source-inventory.v2");
    assert.equal(report.source.commit, COMMIT);
    assert.equal(report.source.tree, TREE);
    assert.deepEqual(report.missingCategories, []);
    assert.deepEqual(report.missingRows, []);
    assert.equal(report.summary.allowedReleaseDeployment, 0);
    assert.equal(report.summary.requiredAutomaticShutdown, 0);
    assert.equal(report.summary.pendingReviewedPredicates, 3);
    assert.deepEqual(report.reviewedPredicates.map(({ status }) => status), ["pending_independent_review", "pending_independent_review", "pending_independent_review"]);
    assert.equal(report.summary.unreviewed, report.summary.candidateCount);
    assert.equal(report.semanticReviews.length, DEFAULT_SEMANTIC_DISPOSITIONS.length);
    assert.ok(report.semanticReviews.every(review => review.status === "pending_source_review"));
    assert.equal(report.status, "fail", "source-role hints and historical identities do not approve semantic dispositions");
    assert.ok(report.entries.some((entry) => entry.category === "managed_env_selector"));
    assert.ok(report.entries.some((entry) => entry.category === "cleanup_selector"));
  });
});

test("production and test sources are scanned while this inventory tool is excluded", () => {
  withFixture({}, (fixture) => {
    const ledgerPath = "apps/cli/scripts/lib/managed-parity-semantic-reviews.mjs";
    fixture.addFile(ledgerPath, 'const selector = "CHARIOX_MANAGED_REVIEW_DATA";\n');
    const report = collect(fixture);
    assert.ok(!report.entries.some(entry => entry.path === ledgerPath), "MP-11 review data is part of the tool, not audited runtime source");
    assert.ok(report.entries.some((entry) => entry.path === "apps/cli/scripts/live-managed-selector.sh"
      && entry.selector === "CHARIOX_MANAGED_CLI_SCRIPT_SELECTOR"));
    assert.equal(report.entries.some((entry) => entry.path === "apps/cli/scripts/managed-parity-source-inventory.mjs"), false);
    const inlineTest = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_TEST_LITERAL");
    assert.equal(inlineTest?.sourceRoleHints.testRegion, "cfg_test");
    assert.equal(inlineTest?.semanticDisposition.status, "unreviewed");
    assert.equal(inlineTest?.semanticDisposition.gateEffect, "fail_closed");
    const inlineTestBwrap = report.entries.find((entry) => entry.category === "bubblewrap"
      && entry.path.endsWith("managed_isolation.rs")
      && entry.sourceRoleHints.testRegion === "cfg_test");
    assert.ok(inlineTestBwrap, "test-only Bubblewrap literals remain lexical candidates");
    assert.equal(inlineTestBwrap.semanticDisposition.status, "unreviewed");
    assert.ok(report.entries.some((entry) => entry.path === "apps/kernel/src/tests/false.rs"
      && entry.sourceRoleHints.testRegion === "test_source"));
  });
});

test("out-of-line cfg(test) module ends before a following production managed branch", () => {
  withFixture({}, (fixture) => {
    const path = "apps/kernel/src/runtime/state.rs";
    fixture.addFile(path, [
      "#[cfg(test)]",
      "mod managed_state_tests",
      ";",
      "",
      "impl ManagedState {",
      "    fn resolve(&self, is_managed: bool) {",
      "        if is_managed {",
      "            let selector = std::env::var(\"CHARIOX_MANAGED_AFTER_SPLIT_MODULE\");",
      "        }",
      "    }",
      "}",
      "",
      "#[cfg(test)]",
      "pub(crate) mod commented_state_tests /* { ; */ // { ;",
      ";",
      "impl CommentedManagedState {",
      "    fn resolve(&self, is_managed: bool) {",
      "        if is_managed {",
      "            let selector = std::env::var(\"CHARIOX_MANAGED_AFTER_COMMENTED_MODULE\");",
      "        }",
      "    }",
      "}",
      "",
      "#[cfg(test)]",
      "mod inline_state_tests {",
      "    const INLINE_TERMINATORS: &str = \"} ; {\";",
      "}",
      "impl InlineManagedState {",
      "    fn resolve(&self, is_managed: bool) {",
      "        if is_managed {",
      "            let selector = std::env::var(\"CHARIOX_MANAGED_AFTER_INLINE_TEST\");",
      "        }",
      "    }",
      "}",
    ].join("\n") + "\n");

    const lifetimePath = "apps/kernel/src/runtime/lifetimes.rs";
    fixture.addFile(lifetimePath, [
      "#[cfg(test)]",
      "mod lifetime_tests",
      ";",
      "impl<'state> LifetimeManagedState<'state> {",
      "    fn resolve(&'state self, is_managed: bool) {",
      "        if is_managed {",
      "            let selector = std::env::var(\"CHARIOX_MANAGED_AFTER_LIFETIME_MODULE\");",
      "        }",
      "    }",
      "}",
    ].join("\n") + "\n");

    const report = collect(fixture);
    for (const [sourcePath, branchCount] of [[path, 3], [lifetimePath, 1]]) {
      const productionCandidates = report.entries.filter((entry) => entry.path === sourcePath
        && ["managed_only_branch", "managed_env_selector"].includes(entry.category));
      assert.deepEqual(
        productionCandidates.map(({ category }) => category).sort(),
        [
          ...Array(branchCount).fill("managed_env_selector"),
          ...Array(branchCount * 2).fill("managed_only_branch"),
        ].sort(),
        "production branches and selectors after the out-of-line declaration must remain candidates",
      );
      for (const candidate of productionCandidates) {
        assert.equal(candidate.sourceRoleHints.testRegion, "none_detected");
        assert.equal(candidate.semanticDisposition.status, "unreviewed");
        assert.equal(candidate.semanticDisposition.gateEffect, "fail_closed");
      }
    }
    assert.equal(report.status, "fail", "module syntax and source-role hints must not approve findings");
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

test("the source inventory can scan committed production while its own report is edited", () => {
  withFixture({ dirtyPath: "docs/MANAGED_PATH1_PARITY_INVENTORY.md" }, (fixture) => {
    assert.doesNotThrow(() => collect(fixture));
  });
});

test("historical approvals stay pending on current source instead of being repinned", () => {
  withFixture({}, (fixture) => {
    const currentCommit = "a".repeat(40);
    const currentTree = "b".repeat(40);
    const runGit = (args, cwd) => {
      if (args[0] === "rev-parse" && args[1] === "HEAD") return `${currentCommit}\n`;
      if (args[0] === "rev-parse" && args[1] === "HEAD^{tree}") return `${currentTree}\n`;
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

test("historical approval identities remain exact and separate from semantic reviews", () => {
  assert.deepEqual(DEFAULT_REVIEWED_PREDICATES, [
    {
      id: "MP-07-release-verify-release",
      sourceCommit: "391b38b2be15c4f49d8ea70cc14b031385cf8331",
      sourceTree: "199b565b83e9582acd38a38a76520e2ba5ac02b4",
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
      sourceCommit: "391b38b2be15c4f49d8ea70cc14b031385cf8331",
      sourceTree: "199b565b83e9582acd38a38a76520e2ba5ac02b4",
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
      sourceCommit: "391b38b2be15c4f49d8ea70cc14b031385cf8331",
      sourceTree: "199b565b83e9582acd38a38a76520e2ba5ac02b4",
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
});

test("semantic review requires the exact source identity, candidate anchor, and independent metadata", () => {
  const source = { commit: COMMIT, tree: TREE };
  const candidate = {
    path: "apps/kernel/src/provider/guard.rs",
    blob: "c".repeat(40),
    line: 12,
    column: 8,
    symbol: "reject_bubblewrap",
    category: "bubblewrap",
    selector: "bwrap",
    contextHash: "d".repeat(64),
  };
  const review = {
    id: "MP-01-guard-review",
    sourceCommit: COMMIT,
    sourceTree: TREE,
    anchor: { ...candidate },
    disposition: "negative_guard",
    independentReview: {
      independent: true,
      reviewer: "independent-reviewer",
      reviewId: "review-2026-09-26-001",
      reviewedAt: "2026-09-26T12:00:00Z",
      rationale: "Source control flow rejects this observed marker before launch.",
    },
  };
  assert.deepEqual(evaluateSemanticDisposition(candidate, source, [review]), {
    status: "reviewed",
    disposition: "negative_guard",
    gateEffect: "reviewed",
    reviewId: "review-2026-09-26-001",
    reviewer: "independent-reviewer",
  });
  assert.equal(evaluateSemanticDisposition(candidate, { ...source, tree: "e".repeat(40) }, [review]).status, "unreviewed");
  assert.equal(evaluateSemanticDisposition({ ...candidate, selector: "bwrap --unshare-user" }, source, [review]).status, "unreviewed");
  assert.equal(evaluateSemanticDisposition(candidate, { ...source, commit: "f".repeat(40) }, [review]).status, "unreviewed");
  assert.throws(() => evaluateSemanticDisposition(candidate, source, [{
    ...review,
    independentReview: { ...review.independentReview, independent: false },
  }]), /invalid independent semantic review metadata/);
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
    assert.ok(report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_TEST_SUFFIX_FALSE_POSITIVE"
      && entry.sourceRoleHints.testRegion === "test_source"));
    assert.equal(report.entries.some((entry) => entry.selector === "CHARIOX_PUBLICATION_CLOUD_API_URL"), false);
    assert.equal(report.entries.some((entry) => entry.selector.includes("COMMENT_ONLY")), false);
    assert.equal(report.entries.some((entry) => entry.path.includes("/generated/")), false);
    assert.ok(report.entries.some((entry) => entry.path.includes("/tests/")
      && entry.sourceRoleHints.testRegion === "test_source"));
    assert.ok(report.entries.some((entry) => entry.path.endsWith("selector.test.rs")
      && entry.sourceRoleHints.testRegion === "test_source"));
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

test("tracked dotfiles use exact reviewed names and unknown hidden files still fail closed", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("apps/ios/.gitignore", "CHARIOX_MANAGED_TRACKED_DOTFILE\n");
    const report = collect(fixture);
    assert.equal(report.entries.some((entry) => entry.path === "apps/ios/.gitignore"), false);

    fixture.addFile("apps/kernel/src/.unknown-managed-source", "CHARIOX_MANAGED_UNKNOWN_DOTFILE\n");
    assert.throws(() => collect(fixture), /unclassified production file: apps\/kernel\/src\/\.unknown-managed-source/);
  });
});

test("unknown positive Path-1 behavior remains an unreviewed fail-closed candidate", () => {
  withFixture({ hiddenManagedFlag: true }, (fixture) => {
    const report = collect(fixture);
    const hidden = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_HIDDEN_FLAG");
    assert.equal(hidden?.sourceRoleHints.topologyHint, "unresolved");
    assert.equal(hidden?.semanticDisposition.status, "unreviewed");
    assert.equal(hidden?.semanticDisposition.gateEffect, "fail_closed");
    assert.equal(report.status, "fail");
  });
});

test("kernel slice broker controls are inventoried separately from provider sandbox selectors", () => {
  withFixture({}, (fixture) => {
    const report = collect(fixture);
    const brokerControls = report.entries.filter((entry) => entry.category === "kernel_slice_broker_control");
    assert.deepEqual(
      brokerControls.map((entry) => entry.selector).sort(),
      [
        "CHARIOX_SLICE_DOCKER_BROKER_FD",
        "CHARIOX_SLICE_DOCKER_BROKER_REQUIRED",
        "CHARIOX_SLICE_DOCKER_BROKER_SOCKET",
        "CHARIOX_SLICE_ROOT",
      ],
    );
    for (const entry of brokerControls) {
      assert.equal(entry.sourceRoleHints.topologyHint, "unresolved");
      assert.equal(entry.semanticDisposition.status, "unreviewed");
      assert.deepEqual(entry.applicableMpIds, ["MP-01", "MP-03", "MP-08", "MP-11"]);
    }
    assert.ok(report.rows.find((row) => row.id === "MP-01")?.categories.includes("kernel_slice_broker_control"));
    assert.ok(report.rows.find((row) => row.id === "MP-03")?.categories.includes("kernel_slice_broker_control"));
    assert.ok(report.rows.find((row) => row.id === "MP-08")?.categories.includes("kernel_slice_broker_control"));
  });
});

test("native release artifact exporter is inventoried separately from release activation", () => {
  withFixture({}, (fixture) => {
    const report = collect(fixture);
    const artifacts = report.entries.filter((entry) => entry.category === "release_artifact_exporter");
    assert.deepEqual(
      artifacts.map((entry) => entry.selector).sort(),
      [
        "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-kernel /chariox-kernel",
        "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-managed-bootstrap /chariox-managed-bootstrap",
        "COPY --from=rust-builder /opt/chariox-source/target/release/chariox-relay /chariox-relay",
        "FROM scratch AS managed-release-artifacts",
      ].sort(),
    );
    assert.ok(artifacts.every((entry) => entry.sourceRoleHints.topologyHint === "inner_docker_slice"));
    assert.ok(artifacts.every((entry) => entry.semanticDisposition.status === "unreviewed"));
    assert.ok(report.rows.find((row) => row.id === "MP-07")?.categories.includes("release_artifact_exporter"));
  });
});

test("current Path-1 source keeps the slice lease in the kernel launch boundary", () => {
  const source = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
  const bootstrap = source("../../../apps/kernel/src/managed_bootstrap/mod.rs");
  const worker = source("../../../apps/kernel/src/managed_bootstrap/worker.rs");
  const supervisor = source("../../../apps/kernel/src/managed_bootstrap/supervisor.rs");
  const broker = source("../../../apps/kernel/src/slice/local_docker/broker.rs");
  const provider = source("../../../apps/kernel/src/provider/managed_isolation.rs");

  assert.match(worker, /super::supervisor::spawn_with_broker_lease\(&mut command,\s*topology\)/);
  const kernelBrokerEnvStart = bootstrap.indexOf("pub(crate) const PATH1_KERNEL_SLICE_BROKER_ENVS");
  const kernelBrokerEnvEnd = bootstrap.indexOf("];", kernelBrokerEnvStart);
  assert.ok(kernelBrokerEnvStart >= 0 && kernelBrokerEnvEnd > kernelBrokerEnvStart);
  const kernelBrokerEnvs = bootstrap.slice(kernelBrokerEnvStart, kernelBrokerEnvEnd);
  for (const name of [
    "CHARIOX_SLICE_ROOT",
    "CHARIOX_SLICE_DOCKER_BROKER_SOCKET",
    "CHARIOX_SLICE_DOCKER_BROKER_FD",
    "CHARIOX_SLICE_DOCKER_BROKER_REQUIRED",
  ]) {
    assert.ok(kernelBrokerEnvs.includes(`"${name}"`), `${name} must be cleared before kernel launch`);
  }
  assert.match(supervisor, /fn broker_share_root_from_socket\(\)\s*-> Option<std::path::PathBuf>\s*\{[\s\S]*?if !socket\.is_absolute\(\)[\s\S]*?socket\s*\.parent\(\)\?\s*\.parent\(\)\?\s*\.parent\(\)/);
  assert.match(supervisor, /fn path1_managed_slice_root_from_broker_socket\(\)\s*-> Option<std::path::PathBuf>\s*\{\s*broker_share_root_from_socket\(\)\.map\(\|share_root\| share_root\.join\("slices"\)\)/);
  assert.match(supervisor, /for name in PATH1_KERNEL_SLICE_BROKER_ENVS\s*\{\s*command\.env_remove\(name\);/);
  assert.match(supervisor, /path1_managed_slice_root_from_broker_socket\(\)\s*\{\s*command\.env\("CHARIOX_SLICE_ROOT",\s*slice_root\);/);
  assert.match(supervisor, /command\s*\.env_remove\(BROKER_SOCKET_ENV\)\s*\.env_remove\(BROKER_REQUIRED_ENV\)\s*\.env\(BROKER_FD_ENV,\s*fd\.to_string\(\)\)/);
  assert.match(supervisor, /command\s*\.env_remove\(BROKER_SOCKET_ENV\)\s*\.env_remove\(BROKER_FD_ENV\)\s*\.env\(BROKER_REQUIRED_ENV,\s*"1"\)/);

  for (const [name, brokerEnv] of [
    ["CHARIOX_SLICE_DOCKER_BROKER_SOCKET", "BROKER_SOCKET_ENV"],
    ["CHARIOX_SLICE_DOCKER_BROKER_FD", "BROKER_FD_ENV"],
    ["CHARIOX_SLICE_DOCKER_BROKER_REQUIRED", "BROKER_REQUIRED_ENV"],
  ]) {
    assert.ok(provider.includes(`"${name}"`), `${name} must be in the provider control scrub`);
    assert.ok(broker.includes(`std::env::remove_var(${brokerEnv})`), `${brokerEnv} must be consumed at kernel startup`);
  }
  assert.ok(provider.includes('"CHARIOX_SLICE_ROOT"'), "provider children must not receive the slice root");
  assert.match(provider, /if !managed_provider_isolation_required\(\)[\s\S]*?managed_provider_control_env_remove\(\)/);
  assert.match(provider, /for name in launch\.pty_env_remove\s*\{\s*command\.env_remove\(name\);/);
  assert.match(broker, /set_close_on_exec\(raw_fd\)/);
});

test("verification guard literals, selected Path-1 directives, and inner-slice Bubblewrap stay distinct", () => {
  withFixture({ directBwrap: true }, (fixture) => {
    const report = collect(fixture);
    const guard = report.entries.find((entry) => entry.category === "bubblewrap" && entry.path.endsWith("verify-image-release.mjs"));
    assert.equal(guard?.sourceRoleHints.executionRole, "verification_guard_candidate");
    assert.equal(guard?.sourceRoleHints.verificationGuardCandidate, true);
    assert.equal(guard?.semanticDisposition.status, "unreviewed");
    assert.equal(guard?.semanticDisposition.gateEffect, "fail_closed");

    const direct = report.entries.find((entry) => entry.category === "bubblewrap" && entry.path.endsWith("provider.service"));
    assert.equal(direct?.sourceRoleHints.selectedServiceTopology, "path1");
    assert.equal(direct?.sourceRoleHints.executionRole, "path1_service_directive_candidate");
    assert.equal(direct?.sourceRoleHints.positivePath1Directive, true);
    assert.equal(direct?.semanticDisposition.status, "unreviewed");
    assert.equal(direct?.semanticDisposition.gateEffect, "fail_closed");

    const docker = report.entries.find((entry) => entry.category === "bubblewrap"
      && entry.sourceRoleHints.topologyHint === "inner_docker_slice");
    assert.equal(docker?.semanticDisposition.status, "unreviewed");
    assert.equal(docker?.path, "apps/kernel/slice-linux-docker/docker/Dockerfile");

    const sharedHostRestriction = report.entries.find((entry) => entry.category === "managed_service_restriction"
      && entry.path.endsWith("chariox-managed-bootstrap.service"));
    assert.equal(sharedHostRestriction?.sourceRoleHints.selectedServiceTopology, "shared_host");
    assert.equal(sharedHostRestriction?.sourceRoleHints.positivePath1Directive, false);
    assert.equal(sharedHostRestriction?.semanticDisposition.status, "unreviewed");
  });
});

test("an explicit Path-1 systemd restriction remains an unreviewed positive directive", () => {
  withFixture({ inheritedRestriction: true }, (fixture) => {
    const report = collect(fixture);
    const restriction = report.entries.find((entry) => entry.selector === "RestrictAddressFamilies");
    assert.equal(restriction?.category, "managed_service_restriction");
    assert.equal(restriction?.sourceRoleHints.selectedServiceTopology, "path1");
    assert.equal(restriction?.sourceRoleHints.positivePath1Directive, true);
    assert.equal(restriction?.semanticDisposition.status, "unreviewed");
  });
});

test("protected-parent filtering remains unreviewed MP-02/MP-03/MP-11 evidence", () => {
  withFixture({ protectedParent: true }, (fixture) => {
    const report = collect(fixture);
    const protectedFinding = report.entries.find((entry) => entry.category === "protected_path_filter");
    assert.deepEqual(protectedFinding?.applicableMpIds, ["MP-02", "MP-03", "MP-11"]);
    assert.equal(protectedFinding?.semanticDisposition.status, "unreviewed");
  });
});

test("an unknown client projection is unreviewed rather than allowed by its filename", () => {
  withFixture({ unknownProjection: true }, (fixture) => {
    const report = collect(fixture);
    const projection = report.entries.find((entry) => entry.path === "apps/client/unknown.ts");
    assert.equal(projection?.category, "client_projection");
    assert.equal(projection?.sourceRoleHints.topologyHint, "unresolved");
    assert.equal(projection?.semanticDisposition.status, "unreviewed");
  });
});

test("forged and stale caller claims cannot supply independent semantic approvals", () => {
  withFixture({}, (fixture) => {
    assert.throws(() => collect(fixture, {
      claimedDispositions: [{
        id: "forged-release-exemption",
        sourceCommit: COMMIT,
        sourceTree: TREE,
        anchor: {
          path: "apps/kernel/src/provider/evil.rs",
          blob: "c".repeat(40),
          line: 1,
          column: 1,
          symbol: "evil",
          category: "managed_env_selector",
          selector: "CHARIOX_MANAGED_FORGED",
          contextHash: "d".repeat(64),
        },
        disposition: "allowed_release_deployment",
        independentReview: {
          independent: true,
          reviewer: "unconfigured-reviewer",
          reviewId: "forged-review-1",
          reviewedAt: "2026-09-26T00:00:00Z",
          rationale: "A caller supplied this approval without a configured review record.",
        },
      }],
    }), /unverified independent semantic disposition claim/);

    assert.throws(() => collect(fixture, {
      claimedDispositions: [{
        id: "MP-07-release-verify-release",
        sourceCommit: PRIOR_REVIEWED_SOURCE_COMMIT,
        sourceTree: PRIOR_REVIEWED_SOURCE_TREE,
        anchor: { path: "apps/kernel/src/managed_bootstrap/release.rs", line: 39 },
        disposition: "allowed_release_deployment",
        independentReview: {
          independent: true,
          reviewer: "unconfigured-reviewer",
          reviewId: "stale-review-1",
          reviewedAt: "2025-01-01T00:00:00Z",
          rationale: "This old approval does not bind the current semantic review anchor.",
        },
      }],
    }), /unverified independent semantic disposition claim/);
  });
});

test("selector topology drift changes only a hint and never carries semantic approval", () => {
  withFixture({ path1ServiceTopology: "shared_host" }, (fixture) => {
    const report = collect(fixture);
    const restriction = report.entries.find((entry) => entry.selector === "ProtectSystem"
      && entry.path.endsWith("provider.service"));
    assert.equal(restriction?.sourceRoleHints.selectedServiceTopology, "shared_host");
    assert.equal(restriction?.sourceRoleHints.positivePath1Directive, false);
    assert.equal(restriction?.semanticDisposition.status, "unreviewed");
    assert.equal(report.status, "fail");
  });
});

test("service path prefixes do not imply Path-1 without an explicit topology marker", () => {
  withFixture({ path1ServiceTopology: null }, (fixture) => {
    const report = collect(fixture);
    const restriction = report.entries.find((entry) => entry.selector === "ProtectSystem"
      && entry.path.endsWith("provider.service"));
    assert.equal(restriction?.path, "deploy/managed-kernel/provider.service");
    assert.equal(restriction?.sourceRoleHints.topologyHint, "unresolved");
    assert.equal(restriction?.sourceRoleHints.selectedServiceTopology, null);
    assert.equal(restriction?.semanticDisposition.gateEffect, "fail_closed");
  });
});

test("source drift keeps the exact historical release predicate pending", () => {
  withFixture({}, (fixture) => {
    const releasePath = join(fixture.root, "apps/kernel/src/managed_bootstrap/release.rs");
    const original = readFileSync(releasePath, "utf8");
    writeFileSync(releasePath, original.replace("pub(super) fn verify_release(", "pub(super) fn verify_release( // drift"));
    const report = collect(fixture);
    const drifted = report.entries.find((entry) => entry.path.endsWith("release.rs") && entry.line === 39);
    assert.equal(drifted?.semanticDisposition.status, "unreviewed");
    assert.equal(report.summary.allowedReleaseDeployment, 0);
    assert.ok(report.reviewedPredicates.find(({ id }) => id === "MP-07-release-verify-release")?.status === "source_drift");
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

test("unified patches retain active and removed selectors with embedded source roles", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("deploy/openship/patches/fixture.patch", [
      "diff --git a/apps/api/src/host.ts b/apps/api/src/host.ts",
      "index 1111111..2222222 100644",
      "--- a/apps/api/src/host.ts",
      "+++ b/apps/api/src/host.ts",
      "@@ -1,2 +1,2 @@",
      "-const old = \"bwrap CHARIOX_MANAGED_REMOVED\";",
      "+const next = \"CHARIOX_OPENSHIP_RUNTIME_PROFILE\";",
      " const ordinary = true;",
      "diff --git a/apps/api/test/host.test.ts b/apps/api/test/host.test.ts",
      "index 1111111..2222222 100644",
      "--- a/apps/api/test/host.test.ts",
      "+++ b/apps/api/test/host.test.ts",
      "@@ -0,0 +1 @@",
      "+const fixture = \"CHARIOX_MANAGED_PATCH_TEST\";",
    ].join("\n") + "\n");
    const report = collect(fixture);
    const removed = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_REMOVED");
    assert.equal(removed?.patchSource.change, "removed");
    assert.equal(removed?.patchSource.path, "apps/api/src/host.ts");
    assert.equal(removed?.patchSource.oldLine, 1);
    assert.equal(removed?.sourceRoleHints.positivePath1Directive, false);
    const added = report.entries.find((entry) => entry.selector === "CHARIOX_OPENSHIP_RUNTIME_PROFILE");
    assert.equal(added?.patchSource.change, "added");
    assert.equal(added?.patchSource.newLine, 1);
    assert.equal(added?.line, 7);
    assert.equal(added?.semanticDisposition.status, "unreviewed");
    const evidence = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_PATCH_TEST");
    assert.equal(evidence?.sourceRoleHints.testRegion, "test_source");
    assert.equal(report.status, "fail");
  });
});

test("manual release candidates retain exact ownership and resource controls without approving them", () => {
  withFixture({}, (fixture) => {
    const path = "deploy/managed-kernel/extract-release.py";
    fixture.addFile(path, [
      "def path_name(name):",
      "    return name",
      "def space_available(path, bytes_needed, inodes_needed, limits):",
      "    pass",
      "def validate(archive, limits, block_size):",
      "    pass",
      "def extract_release(source, destination, limits=Limits(), publication=None):",
      "    pass",
    ].join("\n") + "\n");
    const report = collect(fixture);
    const candidates = report.entries.filter((entry) => entry.path === path);
    assert.equal(candidates.length, 4);
    assert.ok(candidates.every((entry) => entry.candidateOrigin === "manual_source_rule"));
    assert.ok(candidates.every((entry) => entry.sourceClassification.status === "source_drift"));
    assert.ok(candidates.every((entry) => entry.semanticDisposition.status === "unreviewed"));
    assert.equal(report.status, "fail");
  });
});


test("an inspected blob gets only provisional grouping and never independent approval", () => {
  withFixture({}, (fixture) => {
    const rule = SOURCE_AUDIT_RULES.find((rule) => rule.id === "bounded-release-archive-admission");
    const source = readFileSync(new URL("../../../deploy/managed-kernel/extract-release.py", import.meta.url), "utf8");
    fixture.addFile(rule.path, source, "100644", rule.blob);
    const report = collect(fixture);
    const group = report.sourceClassifications.find((group) => group.ruleId === rule.id);
    assert.equal(group.status, "source_inspected");
    assert.equal(group.classification, "signed_release_deployment_control");
    assert.equal(group.authoritative, false);
    assert.equal(group.independentDisposition, "pending");
    assert.equal(group.candidateIds.length, 4);
    assert.ok(report.entries.filter((entry) => group.candidateIds.includes(entry.candidateId))
      .every((entry) => entry.semanticDisposition.gateEffect === "fail_closed"));
    assert.equal(report.status, "fail");
    assert.equal(report.summary.allowedReleaseDeployment, 0);
    assert.equal(report.inventoryTool.modules.length, 9);
    assert.match(report.inventoryTool.bundleSha256, /^[a-f0-9]{64}$/);
  });
});

test("malformed patches and unknown embedded Cloud production formats fail closed", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("deploy/openship/patches/broken.patch", [
      "diff --git a/apps/api/src/host.ts b/apps/api/src/host.ts",
      "--- a/apps/api/src/host.ts", "+++ b/apps/api/src/host.ts",
      "@@ -0,0 +1,2 @@", "+const selector = \"CHARIOX_MANAGED_PATCH\";",
    ].join("\n") + "\n");
    assert.throws(() => collect(fixture), /(?:incomplete patch hunk|invalid patch line)/);
  });
  withFixture({}, (fixture) => {
    fixture.addFile("deploy/openship/patches/unknown.patch", [
      "diff --git a/apps/api/src/host.selector b/apps/api/src/host.selector",
      "--- a/apps/api/src/host.selector", "+++ b/apps/api/src/host.selector",
      "@@ -0,0 +1 @@", "+CHARIOX_MANAGED_PATCH",
    ].join("\n") + "\n");
    assert.throws(() => collect(fixture), /unclassified production file: apps\/api\/src\/host.selector/);
  });
});



test("deployment formats strip comments while unverified fragments remain conservative", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("packages/tool-display/src/index-fragments/part-001.tsfrag", [
      '// CHARIOX_MANAGED_FRAGMENT_COMMENT',
      'const selector = "CHARIOX_MANAGED_FRAGMENT";',
    ].join("\n") + "\n");
    fixture.addFile("scripts/control-drill-fragments/part-001.mjsfrag", 'const selector = "CHARIOX_MANAGED_JS_FRAGMENT";\n');
    fixture.addFile("deploy/production/control-edge.Caddyfile", [
      '# CHARIOX_MANAGED_CADDY_COMMENT',
      'header X-Chariox "CHARIOX_MANAGED_EDGE"',
    ].join("\n") + "\n");
    fixture.addFile("packages/db/prisma/migrations/20260930000000_controls/migration.sql", [
      '-- CHARIOX_MANAGED_SQL_COMMENT',
      '/* CHARIOX_MANAGED_SQL_BLOCK */',
      "SELECT 'CHARIOX_MANAGED_SQL_VALUE';",
    ].join("\n") + "\n");
    const report = collect(fixture);
    const selectors = report.entries.map((entry) => entry.selector);
    for (const selector of ["CHARIOX_MANAGED_FRAGMENT", "CHARIOX_MANAGED_JS_FRAGMENT", "CHARIOX_MANAGED_EDGE", "CHARIOX_MANAGED_SQL_VALUE"])
      assert.ok(selectors.includes(selector), selector);
    assert.ok(!selectors.some((selector) => ["CHARIOX_MANAGED_CADDY_COMMENT", "CHARIOX_MANAGED_SQL_COMMENT", "CHARIOX_MANAGED_SQL_BLOCK"].includes(selector)));
    const unknown = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_FRAGMENT_COMMENT");
    assert.equal(unknown?.sourceRoleHints.lexicalContext, "unknown_fragment_assembly");
    assert.equal(report.status, "fail");
  });
});

test("removed patch block comments cannot hide active added selectors", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("deploy/openship/patches/comment-transition.patch", [
      "diff --git a/apps/api/src/control.ts b/apps/api/src/control.ts",
      "--- a/apps/api/src/control.ts", "+++ b/apps/api/src/control.ts",
      "@@ -1,2 +1 @@",
      "-/* CHARIOX_MANAGED_REMOVED_COMMENT",
      '+const selector = "CHARIOX_MANAGED_PATCH_ACTIVE";',
      '-*/',
    ].join("\n") + "\n");
    const report = collect(fixture);
    const active = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_PATCH_ACTIVE");
    assert.equal(active?.patchSource.change, "added");
    const uncertain = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_REMOVED_COMMENT");
    assert.equal(uncertain?.sourceRoleHints.lexicalContext, "unknown_patch_fragment");
    assert.equal(uncertain?.semanticDisposition.status, "unreviewed");
  });
});

for (const [side, selector] of [["added", "CHARIOX_MANAGED_ADDED_GAP"], ["removed", "CHARIOX_MANAGED_REMOVED_GAP"]]) {
  test("omitted inter-hunk comment closure cannot hide " + side + " patch source", () => {
    withFixture({}, (fixture) => {
      fixture.addFile("deploy/openship/patches/lexical-gap.patch", [
        "diff --git a/apps/api/src/control.ts b/apps/api/src/control.ts",
        "--- a/apps/api/src/control.ts", "+++ b/apps/api/src/control.ts",
        "@@ -1 +1 @@", "-/* old comment", "+/* new comment",
        "@@ -20 +20 @@",
        '-const selector = "' + (side === "removed" ? selector : "ordinary") + '";',
        '+const selector = "' + (side === "added" ? selector : "ordinary") + '";',
      ].join("\n") + "\n");
      const report = collect(fixture);
      const entry = report.entries.find((entry) => entry.selector === selector);
      assert.ok(entry, "unknown omitted lexical context must retain a candidate");
      assert.equal(entry.patchSource.change, side);
      assert.equal(entry.patchSource[side === "added" ? "newLine" : "oldLine"], 20);
      assert.equal(entry.semanticDisposition.status, "unreviewed");
      assert.equal(report.status, "fail");
    });
  });
}


for (const [side, selector] of [["added", "CHARIOX_MANAGED_ADDED_TEMPLATE"], ["removed", "CHARIOX_MANAGED_REMOVED_TEMPLATE"]]) {
  test("unknown starting template context preserves " + side + " patch selectors", () => {
    withFixture({}, (fixture) => {
      const oldLines = ["/* old banner", "  `;", 'const selector = "' + (side === "removed" ? selector : "ordinary") + '";'];
      const newLines = ["/* new banner", "    `;", 'const selector = "' + (side === "added" ? selector : "ordinary") + '";'];
      for (const lines of [oldLines, newLines]) {
        const complete = [...Array(18).fill(""), "const banner = `", ...lines, "selector;"].join("\n");
        assert.equal(runInNewContext(complete), lines === oldLines && side === "removed" || lines === newLines && side === "added" ? selector : "ordinary");
      }
      fixture.addFile("deploy/openship/patches/template-gap.patch", [
        "diff --git a/apps/api/src/control.ts b/apps/api/src/control.ts",
        "--- a/apps/api/src/control.ts", "+++ b/apps/api/src/control.ts",
        "@@ -20,3 +20,3 @@",
        ...oldLines.flatMap((line, index) => ["-" + line, "+" + newLines[index]]),
      ].join("\n") + "\n");
      const report = collect(fixture);
      const entry = report.entries.find((entry) => entry.selector === selector);
      assert.ok(entry, "a valid source template beginning outside the hunk cannot hide executable source");
      assert.equal(entry.patchSource.change, side);
      assert.equal(entry.sourceRoleHints.lexicalContext, "unknown_patch_fragment");
      assert.equal(entry.semanticDisposition.status, "unreviewed");
      assert.equal(report.status, "fail");
    });
  });
}

function fragmentFixture(fixture, suffix, fragments) {
  const directory = suffix === ".tsfrag" ? "packages/tool-display/src/index-fragments" : "scripts/browser-relay-kernel-drill-fragments";
  const dependencies = suffix === ".tsfrag"
    ? [["packages/tool-display/scripts/build-fragments.mjs", "ae72d6fae1ef1e957a39c01417eeb5d7efefcc6b"]]
    : [["scripts/lib/run-fragmented-script.mjs", "0e84d48c7be6e26343039fce0205e9653fc72311"], ["scripts/browser-relay-kernel-drill.mjs", "ad640a15fb5bb5019f5942d378e49403761104e8"]];
  for (const [path, blob] of dependencies) fixture.addFile(path, "// fixture assembler identity\n", "100644", blob);
  for (let index = fragments.length - 1; index >= 0; index--)
    fixture.addFile(directory + "/part-" + String(index + 1).padStart(3, "0") + suffix, fragments[index]);
  return directory;
}

for (const suffix of [".tsfrag", ".mjsfrag"]) {
  test("assembled " + suffix + " retains lexical state and physical source anchors", () => {
    withFixture({}, (fixture) => {
      const fragments = ["const banner = `\n", '/* banner text\n`;\nconst selector = "CHARIOX_MANAGED_FRAGMENT_CONTEXT";\nselector;'];
      assert.equal(runInNewContext(fragments.join("")), "CHARIOX_MANAGED_FRAGMENT_CONTEXT");
      const directory = fragmentFixture(fixture, suffix, fragments);
      const report = collect(fixture);
      const entry = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_FRAGMENT_CONTEXT");
      assert.equal(entry?.path, directory + "/part-002" + suffix);
      assert.equal(entry?.line, 3);
      assert.equal(entry?.fragmentSource.assemblyStatus, "verified_sort_join_empty");
      assert.equal(entry?.semanticDisposition.status, "unreviewed");
    });
  });

  test("assembled " + suffix + " captures a selector split between files", () => {
    withFixture({}, (fixture) => {
      const fragments = ['const selector = "CHARIOX_MANA', 'GED_JOINED";\nselector;'];
      assert.equal(runInNewContext(fragments.join("")), "CHARIOX_MANAGED_JOINED");
      const directory = fragmentFixture(fixture, suffix, fragments);
      const report = collect(fixture);
      const entry = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_JOINED");
      assert.equal(entry?.path, directory + "/part-001" + suffix);
      assert.equal(entry?.line, 1);
      assert.deepEqual(entry?.fragmentSource.matchSegments.map((segment) => segment.path),
        [directory + "/part-001" + suffix, directory + "/part-002" + suffix]);
      assert.equal(entry?.semanticDisposition.status, "unreviewed");
    });
  });
}

test("unverified fragment assembly is an explicit unresolved source gap", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("apps/api/src/unknown-fragments/part-001.tsfrag", '/* maybe a literal\nconst value = "CHARIOX_MANAGED_UNKNOWN_FRAGMENT";');
    const report = collect(fixture);
    assert.ok(report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_UNKNOWN_FRAGMENT"));
    assert.ok(report.sourceAuditGaps.some((gap) => gap.kind === "fragment_assembly_unresolved"));
    assert.equal(report.status, "fail");
  });
});

test("the real generic release preparation control gets a manual candidate", () => {
  withFixture({}, (fixture) => {
    const rule = SOURCE_AUDIT_RULES.find((rule) => rule.id === "release-update-outcome-evidence");
    const source = readFileSync(new URL("../../../apps/kernel/src/runtime/managed_release_update_evidence.rs", import.meta.url), "utf8");
    fixture.addFile(rule.path, source, "100644", rule.blob);
    const report = collect(fixture);
    const entry = report.entries.find((entry) => entry.path === rule.path && entry.symbol === "prepare_archive" && entry.candidateOrigin === "manual_source_rule");
    assert.ok(entry);
    assert.equal(entry.semanticDisposition.status, "unreviewed");
  });
});

test("an expected manual declaration that disappears is an unresolved audit gap", () => {
  withFixture({}, (fixture) => {
    const rule = SOURCE_AUDIT_RULES.find((rule) => rule.id === "release-update-outcome-evidence");
    const source = readFileSync(new URL("../../../apps/kernel/src/runtime/managed_release_update_evidence.rs", import.meta.url), "utf8");
    fixture.addFile(rule.path, source.replaceAll("prepare_archive", "renamed_archive"));
    const report = collect(fixture);
    assert.ok(report.sourceAuditGaps.some((gap) => gap.ruleId === rule.id && gap.symbol === "prepare_archive" && gap.kind === "expected_declaration_missing"));
    assert.equal(report.status, "fail");
  });
});

test("a scoped runtime audit does not classify unrelated inline tests", () => {
  withFixture({}, (fixture) => {
    const rule = SOURCE_AUDIT_RULES.find((rule) => rule.id === "official-provider-adapter-wiring");
    const source = readFileSync(new URL("../../../apps/kernel/src/provider/registry.rs", import.meta.url), "utf8");
    fixture.addFile(rule.path, source, "100644", rule.blob);
    const report = collect(fixture);
    const entries = report.entries.filter((entry) => entry.path === rule.path);
    assert.ok(entries.some((entry) => entry.sourceClassification?.status === "source_inspected"));
    const tests = entries.filter((entry) => entry.sourceRoleHints.testRegion === "cfg_test");
    assert.ok(tests.length > 0);
    assert.ok(tests.every((entry) => entry.sourceClassification === null));
    assert.ok(entries.every((entry) => entry.semanticDisposition.status === "unreviewed"));
    const group = report.sourceClassifications.find((group) => group.ruleId === rule.id);
    assert.deepEqual(group.auditedRanges, [[1, 259]]);
  });
});

test("slice placement observations remain separate from independent removal findings", () => {
  withFixture({}, (fixture) => {
    const rule = SOURCE_AUDIT_RULES.find((rule) => rule.id === "machine-scoped-slice-creation-and-placement");
    const source = readFileSync(new URL("../../../apps/kernel/src/slice/store.rs", import.meta.url), "utf8");
    fixture.addFile(rule.path, source, "100644", rule.blob);
    const report = collect(fixture);
    const group = report.sourceClassifications.find((group) => group.ruleId === rule.id);
    assert.equal(group.classification, "shared_slice_identity_namespace");
    assert.equal(group.status, "source_inspected");
    assert.ok(group.openFindings.some((finding) => finding.includes("Fresh ordinary/Path-1 runtime comparison")));
    assert.equal(group.independentDisposition, "pending");
    assert.equal(report.summary.removalRequired, 0);
    assert.equal(report.status, "fail");
  });
});

test("manual audit anchors include generic utility resource functions", () => {
  withFixture({}, (fixture) => {
    const rule = SOURCE_AUDIT_RULES.find((rule) => rule.id === "shared-provider-utility-capture-budget");
    const source = readFileSync(new URL("../../../apps/kernel/src/local/provider_requests/catalog/probe_capture.rs", import.meta.url), "utf8");
    fixture.addFile(rule.path, source, "100644", rule.blob);
    const report = collect(fixture);
    const entries = report.entries.filter((entry) => entry.path === rule.path && entry.candidateOrigin === "manual_source_rule");
    assert.deepEqual(entries.map((entry) => entry.symbol).sort(), ["capture", "drain"]);
    assert.ok(entries.every((entry) => entry.semanticDisposition.status === "unreviewed"));
  });
});

test("machine enrollment data shapes get explicit candidates without widening source review", () => {
  withFixture({}, (fixture) => {
    const rule = SOURCE_AUDIT_RULES.find((rule) => rule.id === "managed-bootstrap-machine-profile-shape");
    const source = readFileSync(new URL("../../../apps/kernel/src/managed_bootstrap/cloud.rs", import.meta.url), "utf8");
    fixture.addFile(rule.path, source, "100644", rule.blob);
    const report = collect(fixture);
    const candidate = report.entries.find((entry) => entry.path === rule.path && entry.symbol === "ManagedCloudRelayProfile");
    assert.equal(candidate?.candidateOrigin, "manual_source_rule");
    assert.equal(candidate?.sourceClassification.status, "source_inspected");
    assert.equal(candidate?.sourceClassification.classification, "machine_bound_deployment_enrollment");
    assert.equal(candidate?.semanticDisposition.status, "unreviewed");
    assert.ok(report.entries.filter((entry) => entry.path === rule.path && entry.line > 64)
      .every((entry) => entry.sourceClassification === null));
  });
});


for (const delimiter of ["$$", "$banner$"]) {
  test("PostgreSQL " + delimiter + " literals cannot hide following SQL selectors", () => {
    withFixture({}, (fixture) => {
      const path = "packages/db/prisma/migrations/20261001000000_dollar/migration.sql";
      fixture.addFile(path, [
        "SELECT " + delimiter + "/* banner text -- still literal" + delimiter + ";",
        "SELECT 'CHARIOX_MANAGED_SQL_AFTER_DOLLAR';",
      ].join("\n") + "\n");
      const report = collect(fixture);
      const entry = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_SQL_AFTER_DOLLAR");
      assert.equal(entry?.path, path);
      assert.equal(entry?.line, 2);
      assert.equal(entry?.semanticDisposition.status, "unreviewed");
      assert.equal(report.status, "fail");
    });
  });
}

test("PostgreSQL dollar quotes close only at the matching tag", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("packages/db/prisma/migrations/20261001000001_tags/migration.sql", [
      "SELECT $banner$other delimiters $$ $other$ /* literal",
      "$banner$;",
      "/* CHARIOX_MANAGED_SQL_ACTUAL_COMMENT */",
      "SELECT 'CHARIOX_MANAGED_SQL_AFTER_MATCHED_TAG';",
    ].join("\n") + "\n");
    const report = collect(fixture);
    assert.ok(report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_SQL_AFTER_MATCHED_TAG" && entry.line === 4));
    assert.ok(!report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_SQL_ACTUAL_COMMENT"));
  });
});

test("PostgreSQL nested block comments do not leak outer comment candidates", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("packages/db/prisma/migrations/20261001000002_nested/migration.sql", [
      "/* outer /* inner */ CHARIOX_MANAGED_SQL_NESTED_COMMENT */",
      "SELECT 'CHARIOX_MANAGED_SQL_AFTER_NESTED';",
    ].join("\n") + "\n");
    const report = collect(fixture);
    assert.ok(report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_SQL_AFTER_NESTED" && entry.line === 2));
    assert.ok(!report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_SQL_NESTED_COMMENT"));
  });
});

test("PostgreSQL E-string escapes keep comment markers inside the literal", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("packages/db/prisma/migrations/20261001000003_escape/migration.sql",
      "SELECT E'escaped\\'/* banner -- still literal';\nSELECT 'CHARIOX_MANAGED_SQL_AFTER_ESCAPE';\n");
    const report = collect(fixture);
    assert.ok(report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_SQL_AFTER_ESCAPE" && entry.line === 2));
  });
});

test("PostgreSQL plain-string backslash settings cannot hide executable SQL", () => {
  withFixture({}, (fixture) => {
    // Valid with standard_conforming_strings=on: the first string contains a
    // literal backslash; the second contains a comment opener.
    fixture.addFile("packages/db/prisma/migrations/20261001000004_plain/migration.sql",
      "SELECT '\\';\nSELECT '/* banner text';\nSELECT 'CHARIOX_MANAGED_SQL_AFTER_PLAIN_BACKSLASH';\n");
    const report = collect(fixture);
    assert.ok(report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_SQL_AFTER_PLAIN_BACKSLASH" && entry.line === 3));
  });
});


test("PostgreSQL dollar characters inside unquoted identifiers do not open literals", () => {
  withFixture({}, (fixture) => {
    const path = "packages/db/prisma/migrations/20261001000005_identifier/migration.sql";
    fixture.addFile(path, [
      "CREATE TABLE foo$tag$(id text DEFAULT '$tag$/*');",
      "SELECT 'CHARIOX_MANAGED_SQL_AFTER_DOLLAR_IDENTIFIER';",
    ].join("\n") + "\n");
    const report = collect(fixture);
    const entry = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_SQL_AFTER_DOLLAR_IDENTIFIER");
    assert.equal(entry?.path, path);
    assert.equal(entry?.line, 2);
    assert.equal(entry?.semanticDisposition.status, "unreviewed");
  });
});


// Pinned declaration excerpts keep scanner tests independent of PR664's runtime
// base. Historical 65d/8e9 reports are retained; refreshed observations require
// exact current-source scans. These fixtures test declaration/range/disposition only.
const CURRENT_DECLARATION_EXCERPTS = {
  "shared-exact-slice-binding-recovery": [
    [45, "pub(crate) async fn execute_remote_agent_binding_refresh("],
    [651, "pub(crate) fn spawn_worker_agent("],
    [1002, "pub(crate) fn prepare_remote_agent_binding_refresh("],
    [1099, "pub(crate) fn refresh_remote_agent_binding("],
    [1171, "pub(crate) fn refresh_remote_agent_binding_to_worker_kernel_with_operation("],
    [1503, "fn select_remote_kernel_by_ref_with_config("],
  ],
  "guarded-friendly-slice-reference-resolution": [[6, "pub(crate) fn resolve_execution_worker_kernel_ref("]],
  "inner-slice-kernel-bootstrap-identity": [[132, "start_slice_kernel() {"]],
  "slice-provisioner-container-identity-forwarding": [[1131, "exec_slice_with_timeout() {"]],
  "path1-broker-prepared-image-admission": [
    [118, "const ALLOWED_ENVIRONMENT = new Set(["],
    [336, "function validateDocker(args) {"],
    [439, "function validateProvisioner(action, environment, files) {"],
  ],
  "per-creation-machine-scoped-slice-ref": [
    [6, "pub(super) fn new_local_docker_worker_ref("],
    [23, "fn worker_ref_with_nonce("],
    [43, "pub(super) fn qualified_worker_ref_parts(worker_ref: &str) -> Option<(&str, &str)> {"],
    [56, "pub(crate) fn machine_scoped_slice_worker_ref(worker_ref: &str, machine_id: &str) -> bool {"],
    [66, "pub(crate) fn require_hosted_slice_worker_ref("],
    [78, 'const OUTSIDE_SCOPE: &str = "CHARIOX_MANAGED_CURRENT_OUTSIDE_SCOPE";'],
  ],
};

function currentDeclarationExcerpt(ruleId) {
  const excerpt = CURRENT_DECLARATION_EXCERPTS[ruleId];
  const lines = Array.from({ length: Math.max(...excerpt.map(([line]) => line)) }, () => "");
  for (const [line, text] of excerpt) lines[line - 1] = text;
  return lines.join("\n") + "\n";
}

for (const [ruleId, symbol] of [
  ["inner-slice-kernel-bootstrap-identity", "start_slice_kernel"],
  ["slice-provisioner-container-identity-forwarding", "exec_slice_with_timeout"],
  ["path1-broker-prepared-image-admission", "ALLOWED_ENVIRONMENT"],
]) {
  test("current source audit anchors " + symbol + " at its pinned declaration", () => {
    withFixture({}, (fixture) => {
      const rule = SOURCE_AUDIT_RULES.find((rule) => rule.id === ruleId);
      fixture.addFile(rule.path, currentDeclarationExcerpt(ruleId), "100644", rule.blob);
      const report = collect(fixture);
      const entry = report.entries.find((entry) => entry.path === rule.path && entry.symbol === symbol && entry.candidateOrigin === "manual_source_rule");
      assert.ok(entry, "an inspected declaration cannot silently lose its manual candidate");
      assert.equal(entry.sourceClassification.status, "source_inspected");
      assert.equal(entry.semanticDisposition.status, "unreviewed");
      assert.ok(!report.sourceAuditGaps.some((gap) => gap.ruleId === ruleId && gap.symbol === symbol));
    });
  });
}

test("current Machine-qualified namespace observations do not approve out-of-range source or drift", () => {
  withFixture({}, (fixture) => {
    const rule = SOURCE_AUDIT_RULES.find((rule) => rule.id === "per-creation-machine-scoped-slice-ref");
    const source = currentDeclarationExcerpt(rule.id);
    fixture.addFile(rule.path, source, "100644", rule.blob);
    const report = collect(fixture);
    const entries = report.entries.filter((entry) => entry.path === rule.path && entry.candidateOrigin === "manual_source_rule");
    assert.deepEqual(entries.map((entry) => entry.symbol).sort(), [
      "machine_scoped_slice_worker_ref", "new_local_docker_worker_ref",
      "qualified_worker_ref_parts", "require_hosted_slice_worker_ref", "worker_ref_with_nonce",
    ]);
    assert.ok(entries.every((entry) => entry.sourceClassification.classification === "shared_slice_identity_namespace"));
    assert.ok(entries.every((entry) => entry.semanticDisposition.status === "unreviewed"));
    assert.equal(report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_CURRENT_OUTSIDE_SCOPE")?.sourceClassification, null);
    withFixture({}, (changedFixture) => {
      changedFixture.addFile(rule.path, source + "\n// changed source identity\n");
      const changed = collect(changedFixture).sourceClassifications.find((group) => group.ruleId === rule.id);
      assert.equal(changed.status, "source_drift");
      assert.equal(changed.classification, null);
    });
  });
});

test("slice context and attachment observations stay scoped and unreviewed", () => {
  withFixture({}, (fixture) => {
    const context = SOURCE_AUDIT_RULES.find((rule) => rule.id === "slice-local-worker-context");
    const attachment = SOURCE_AUDIT_RULES.find((rule) => rule.id === "slice-recorded-attachment-identity");
    assert.equal(context.blob, attachment.blob);
    const lines = Array.from({ length: 173 }, () => "");
    for (const [line, declaration] of [
      [5, "pub(crate) fn slice_worker_id("],
      [28, "pub(crate) fn current_slice_worker_id() -> Option<String> {"],
      [39, "pub(crate) fn slice_worker_id_for_config(config: &DaemonConfig) -> Option<String> {"],
      [52, "enum RecordedSliceWorker<'a> {"],
      [58, "pub(crate) fn recorded_slice_for_worker<'a>("],
      [73, "pub(crate) fn retained_slice_attachment_matches("],
      [95, "fn recorded_relay_is_hosted(config: &DaemonConfig, slice: &SliceRecord) -> bool {"],
      [105, "fn recorded_slice_worker<'a>("],
      [173, 'const OUTSIDE: &str = "CHARIOX_MANAGED_ATTACHMENT_TEST_ONLY";'],
    ]) lines[line - 1] = declaration;
    fixture.addFile(context.path, lines.join("\n") + "\n", "100644", context.blob);
    const report = collect(fixture);
    const manual = report.entries.filter((entry) => entry.path === context.path && entry.candidateOrigin === "manual_source_rule");
    assert.equal(manual.length, 8);
    assert.ok(manual.every((entry) => entry.sourceClassification.status === "source_inspected"));
    assert.ok(manual.every((entry) => entry.semanticDisposition.status === "unreviewed"));
    assert.equal(report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_ATTACHMENT_TEST_ONLY")?.sourceClassification, null);
    assert.ok(!report.sourceAuditGaps.some((gap) => [context.id, attachment.id].includes(gap.ruleId)));
    assert.equal(report.status, "fail");
  });
});

test("current Cloud source anchors remain explicit missing-source gaps", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("apps/api/src/current-cloud.ts", "export const ready = true;\n");
    const report = collect(fixture);
    const gap = report.sourceAuditGaps.find((entry) => entry.ruleId === "canonical-slice-cloud-recovery-shape");
    assert.equal(gap?.kind, "expected_source_missing");
    assert.equal(gap?.auditSourceCommit, "06cd95fda1fc07f9dd37a12727f6ed4e5a4adeb2");
    assert.equal(report.status, "fail");
  });
});

test("corrected source observations still require independent disposition", () => {
  withFixture({}, (fixture) => {
    const ids = ["shared-exact-slice-binding-recovery", "guarded-friendly-slice-reference-resolution"];
    for (const id of ids) {
      const rule = SOURCE_AUDIT_RULES.find((rule) => rule.id === id);
      fixture.addFile(rule.path, CURRENT_DECLARATION_EXCERPTS[id] ? currentDeclarationExcerpt(id)
        : readFileSync(new URL("../../../" + rule.path, import.meta.url), "utf8"), "100644", rule.blob);
    }
    const report = collect(fixture);
    for (const id of ids) {
      const group = report.sourceClassifications.find((entry) => entry.ruleId === id);
      assert.equal(group?.status, "source_inspected");
      assert.ok(group.openFindings.some((finding) => finding.includes("independent semantic disposition remain pending")));
      assert.ok(!group.openFindings.some((finding) => finding.startsWith("Open at OSS 65d")));
      assert.equal(group.independentDisposition, "pending");
    }
    assert.equal(report.summary.removalRequired, 0);
    assert.equal(report.status, "fail");
  });
});


for (const [name, argument] of [
  ["backtick-quoted", '`#{$CHARIOX_MANAGED_CADDY_CONTROL}`'],
  ["within an unquoted token", "prefix#{$CHARIOX_MANAGED_CADDY_CONTROL}"],
]) {
  test("Caddy keeps active selector " + name, () => {
    withFixture({}, (fixture) => {
      fixture.addFile("deploy/production/control-edge.Caddyfile", [
        ":8080 {", "    header X-Selector " + argument, "}", "",
      ].join("\n"));
      const report = collect(fixture);
      const entry = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_CADDY_CONTROL");
      assert.ok(entry, "valid Caddy token contents cannot disappear as comments");
      assert.equal(entry.line, 2);
      assert.equal(entry.semanticDisposition.status, "unreviewed");
      assert.equal(report.status, "fail");
    });
  });
}

test("Caddy token-boundary comments remain absent after quoted and unquoted tokens", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("deploy/production/control-edge.Caddyfile", [
      '# CHARIOX_MANAGED_CADDY_STANDALONE_COMMENT',
      'header X-Value "literal"# CHARIOX_MANAGED_CADDY_AFTER_QUOTE_COMMENT',
      'header X-Value prefix"literal # CHARIOX_MANAGED_CADDY_INSIDE_TOKEN_QUOTE_COMMENT',
      'header X-Value ordinary # CHARIOX_MANAGED_CADDY_AFTER_TOKEN_COMMENT',
      'header X-Value `backtick`# CHARIOX_MANAGED_CADDY_AFTER_BACKTICK_COMMENT',
      'header X-Value "CHARIOX_MANAGED_CADDY_DOUBLE_QUOTED"',
      '',
    ].join("\n"));
    const report = collect(fixture);
    const selectors = report.entries.map((entry) => entry.selector);
    assert.ok(selectors.includes("CHARIOX_MANAGED_CADDY_DOUBLE_QUOTED"));
    assert.ok(!selectors.some((selector) => selector.endsWith("_COMMENT") && selector.includes("_CADDY_")));
  });
});

test("Caddy multiline quotes and heredocs keep literal hash content and physical anchors", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("deploy/production/control-edge.Caddyfile", [
      'header X-Value `first line',
      '# CHARIOX_MANAGED_CADDY_MULTILINE_BACKTICK',
      'last line`',
      'header X-Value "first line',
      '# CHARIOX_MANAGED_CADDY_MULTILINE_DOUBLE',
      'last line"',
      'respond <<BODY',
      '# CHARIOX_MANAGED_CADDY_HEREDOC',
      'literal `quote" text',
      'BODY',
      '# CHARIOX_MANAGED_CADDY_AFTER_HEREDOC_COMMENT',
      '',
    ].join("\n"));
    const report = collect(fixture);
    for (const [selector, line] of [
      ["CHARIOX_MANAGED_CADDY_MULTILINE_BACKTICK", 2],
      ["CHARIOX_MANAGED_CADDY_MULTILINE_DOUBLE", 5],
      ["CHARIOX_MANAGED_CADDY_HEREDOC", 8],
    ]) {
      const entry = report.entries.find((entry) => entry.selector === selector);
      assert.equal(entry?.line, line);
      assert.equal(entry?.semanticDisposition.status, "unreviewed");
    }
    assert.ok(!report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_CADDY_AFTER_HEREDOC_COMMENT"));
  });
});

test("Caddy backtick escapes stay literal while double-quoted escaped quotes remain quoted", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("deploy/production/control-edge.Caddyfile", [
      'header X-Value `literal\\`# CHARIOX_MANAGED_CADDY_BACKTICK_COMMENT',
      'header X-Value "literal\\" # CHARIOX_MANAGED_CADDY_ESCAPED_DOUBLE"',
      'header X-Value prefix\\#{$CHARIOX_MANAGED_CADDY_ESCAPED_PREFIX}',
      '',
    ].join("\n"));
    const report = collect(fixture);
    assert.ok(!report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_CADDY_BACKTICK_COMMENT"));
    assert.equal(report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_CADDY_ESCAPED_DOUBLE")?.line, 2);
    assert.equal(report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_CADDY_ESCAPED_PREFIX")?.line, 3);
  });
});


test("Caddy CR and Unicode token boundaries follow the lexer rather than generic config rules", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("deploy/production/control-edge.Caddyfile", [
      '\ufeffheader X-Value prefix\r#{$CHARIOX_MANAGED_CADDY_CR_TOKEN}',
      'header X-Value prefix\ufeff#{$CHARIOX_MANAGED_CADDY_INTERIOR_BOM}',
      'header X-Value prefix\u00a0# CHARIOX_MANAGED_CADDY_UNICODE_COMMENT',
      '',
    ].join("\n"));
    const report = collect(fixture);
    assert.equal(report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_CADDY_CR_TOKEN")?.line, 1);
    assert.equal(report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_CADDY_INTERIOR_BOM")?.line, 2);
    assert.ok(!report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_CADDY_UNICODE_COMMENT"));
  });
});

test("Caddy escaped heredoc openers keep true comments while uncertain heredocs stay conservative", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("deploy/production/control-edge.Caddyfile", [
      'respond \\<<BODY',
      '# CHARIOX_MANAGED_CADDY_ESCAPED_HEREDOC_COMMENT',
      'BODY',
      'respond <<UNCLOSED',
      '# CHARIOX_MANAGED_CADDY_UNCERTAIN_HEREDOC',
      '',
    ].join("\n"));
    const report = collect(fixture);
    assert.ok(!report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_CADDY_ESCAPED_HEREDOC_COMMENT"));
    const entry = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_CADDY_UNCERTAIN_HEREDOC");
    assert.equal(entry?.line, 5);
    assert.equal(entry?.semanticDisposition.status, "unreviewed");
    assert.equal(report.status, "fail");
  });
});


for (const suffix of [".tsfrag", ".mjsfrag"]) {
  test("assembled " + suffix + " preserves nested templates before active controls", () => {
    withFixture({}, (fixture) => {
      const fragments = ['const banner = `${`/* banner text`}`;\n',
        'const selector = "CHARIOX_MANAGED_NESTED_FRAGMENT";\nselector;'];
      assert.equal(runInNewContext(fragments.join("")), "CHARIOX_MANAGED_NESTED_FRAGMENT");
      const directory = fragmentFixture(fixture, suffix, fragments);
      const report = collect(fixture);
      const entry = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_NESTED_FRAGMENT");
      assert.equal(entry?.path, directory + "/part-002" + suffix);
      assert.equal(entry?.line, 1);
      assert.equal(entry?.sourceRoleHints.lexicalContext, "unparsed_fragment_assembly");
      assert.equal(entry?.fragmentSource.assemblyStatus, "verified_sort_join_empty");
      assert.equal(entry?.semanticDisposition.status, "unreviewed");
    });
  });

  test("assembled " + suffix + " retains regex and apparent-comment candidates conservatively", () => {
    withFixture({}, (fixture) => {
      const fragments = ['const punctuation = /[/*]/;\n',
        '// CHARIOX_MANAGED_FRAGMENT_COMMENT_CANDIDATE\nconst selector = "CHARIOX_MANAGED_REGEX_FRAGMENT";\nselector;'];
      assert.equal(runInNewContext(fragments.join("")), "CHARIOX_MANAGED_REGEX_FRAGMENT");
      const directory = fragmentFixture(fixture, suffix, fragments);
      const report = collect(fixture);
      for (const [selector, line] of [["CHARIOX_MANAGED_FRAGMENT_COMMENT_CANDIDATE", 1], ["CHARIOX_MANAGED_REGEX_FRAGMENT", 2]]) {
        const entry = report.entries.find((entry) => entry.selector === selector);
        assert.equal(entry?.path, directory + "/part-002" + suffix);
        assert.equal(entry?.line, line);
        assert.equal(entry?.sourceRoleHints.lexicalContext, "unparsed_fragment_assembly");
        assert.equal(entry?.semanticDisposition.status, "unreviewed");
      }
      assert.equal(report.fragmentAssemblies[0]?.lexicalContext, "unparsed_fragment_assembly");
      assert.equal(report.status, "fail");
    });
  });
}


for (const quote of ['`', '"']) {
  for (const opener of ['<<END', '<\r<END']) {
    test("Caddy heredoc preserves an adjacent multiline " + JSON.stringify(quote) + " token after " + JSON.stringify(opener), () => {
      withFixture({}, (fixture) => {
        fixture.addFile("deploy/production/control-edge.Caddyfile", [
          ':8080 {', '  map {host} {first} {second} {', '    default ' + opener,
          'body', 'END' + quote + 'first line', 'END',
          '#{$CHARIOX_MANAGED_CADDY_EARLY_END}', 'last line' + quote,
          '# CHARIOX_MANAGED_CADDY_AFTER_ADJACENT_COMMENT',
          '  }', '  respond "{second}"', '}', '',
        ].join('\n'));
        const report = collect(fixture);
        const entry = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_CADDY_EARLY_END");
        assert.equal(entry?.line, 7);
        assert.equal(entry?.semanticDisposition.status, "unreviewed");
        assert.ok(!report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_CADDY_AFTER_ADJACENT_COMMENT"));
      });
    });
  }
}


test("manual shutdown declarations retain MP09 and require independent disposition", () => {
  withFixture({}, (fixture) => {
    const rule = SOURCE_AUDIT_RULES.find((rule) => rule.id === "managed-auto-stop-deadline");
    fixture.addFile(rule.path, "export function normalizeAutoStopPolicy(policy) {}\nexport function managedEnvironmentIdleDeadline(environment, idleAt) {}\n", "100644", rule.blob);
    const report = collect(fixture);
    const entries = report.entries.filter((entry) => entry.path === rule.path && entry.candidateOrigin === "manual_source_rule");
    assert.equal(entries.length, 2);
    for (const entry of entries) {
      assert.ok(entry.applicableMpIds.includes("MP-09"));
      assert.equal(entry.sourceClassification.status, "source_inspected");
      assert.equal(entry.semanticDisposition.status, "unreviewed");
    }
    assert.equal(report.status, "fail");
  });
});


test("JSON policy declaration anchors bind exact data without granting approval", () => {
  const rule = SOURCE_AUDIT_RULES.find((rule) => rule.id === "typed-docker-image-reference-policy-data");
  const source = JSON.stringify({ reference: "synthetic-expression", maximumRepositoryPath: 255, defaultBase: "synthetic:base" }, null, 2) + "\n";
  withFixture({}, (fixture) => {
    fixture.addFile(rule.path, source, "100644", rule.blob);
    const report = collect(fixture);
    const manual = report.entries.filter((entry) => entry.path === rule.path && entry.candidateOrigin === "manual_source_rule");
    assert.deepEqual(manual.map((entry) => entry.symbol).sort(), ["defaultBase", "maximumRepositoryPath", "reference"]);
    assert.ok(manual.every((entry) => entry.sourceClassification.status === "source_inspected"));
    assert.ok(manual.every((entry) => entry.semanticDisposition.status === "unreviewed"));
    assert.ok(!report.sourceAuditGaps.some((gap) => gap.ruleId === rule.id));
    assert.equal(report.status, "fail");
  });
  withFixture({}, (fixture) => {
    fixture.addFile(rule.path, source.replace("255", "256"));
    const group = collect(fixture).sourceClassifications.find((group) => group.ruleId === rule.id);
    assert.equal(group?.status, "source_drift");
    assert.equal(group?.classification, null);
    assert.equal(group?.independentDisposition, "pending");
  });
});

for (const [id, declarations] of [
  ["shared-private-home-archive-dispatch", [[916, "fn archive_local_docker_home_volume() {}"], [958, "fn archive_local_docker_home_volume_with_helper() {}"]]],
  ["ordinary-private-home-archive-stream", [[47, "pub(super) fn capture() {}"], [272, "fn remove_created_archive() {}"]]],
  ["managed-private-home-archive-publication", [[695, "function validateArtifactIdentity() {}"], [719, "async function captureHomeArchive() {}"], [788, "async function inspectManagedHomeArchive() {}"], [827, "async function verifyHomeArchive() {}"], [838, "function removeHomeArchive() {}"]]],
  ["managed-home-archive-stream-policy", [[16, "export function isHomeArchiveRestoreRequest() {}"], [22, "export function homeArchiveMetadataMatches() {}"], [30, "export async function capturePrivateHomeArchive() {}"]]],
]) {
  test("scoped home archive declarations stay unreviewed: " + id, () => {
    const rule = SOURCE_AUDIT_RULES.find((rule) => rule.id === id);
    assert.ok(rule, "archive policy requires an explicit inspected source rule");
    const lines = Array.from({ length: Math.max(...declarations.map(([line]) => line)) }, () => "");
    for (const [line, declaration] of declarations) lines[line - 1] = declaration;
    withFixture({}, (fixture) => {
      fixture.addFile(rule.path, lines.join("\n") + "\n", "100644", rule.blob);
      const report = collect(fixture);
      const manual = report.entries.filter((entry) => entry.candidateOrigin === "manual_source_rule" && entry.sourceClassification?.ruleId === id);
      assert.equal(manual.length, declarations.length);
      assert.ok(manual.every((entry) => entry.sourceClassification.status === "source_inspected"));
      assert.ok(manual.every((entry) => entry.semanticDisposition.status === "unreviewed"));
      assert.ok(manual.every((entry) => entry.sourceClassification.independentDisposition === "pending"));
      assert.ok(!report.sourceAuditGaps.some((gap) => gap.ruleId === id));
      if (id.startsWith("managed-")) {
        assert.ok(manual.every((entry) => entry.sourceClassification.independentDisposition === "pending"));
      }
      assert.equal(report.status, "fail");
    });
    withFixture({}, (fixture) => {
      fixture.addFile(rule.path, lines.join("\n") + "\n");
      const report = collect(fixture);
      const group = report.sourceClassifications.find((group) => group.ruleId === id);
      assert.equal(group?.status, "source_drift");
      assert.equal(group?.classification, null);
      assert.equal(group?.independentDisposition, "pending");
    });
  });
}


// Pinned declaration-only fixtures from the final source; no runtime source is imported.
const FINAL_ARCHIVE_SOURCE = "686ec57d5e46cdd46e723155b7eb89f6f25202a2";
const FINAL_ARCHIVE_DECLARATIONS = [
  {
    "id": "managed-home-archive-digest-policy",
    "path": "apps/kernel/slice-linux-docker/managed-home-archive-digest.mjs",
    "blob": "8ac3933cf9b9d4ade377fb9b8e7857c40ce8488d",
    "declarations": [
      [
        6,
        "export async function digestPinnedHomeArchive() {}"
      ],
      [
        34,
        "function validateProgressTimeout() {}"
      ]
    ],
    "anchorCount": 2,
    "sourceCommit": "3679fbb8bee37152fa694436297f558af2197d2c"
  },
  {
    "id": "shared-home-archive-policy-data",
    "path": "apps/kernel/slice-linux-docker/home-archive-policy.json",
    "blob": "1d54fe7e4eca155a39afdb9a30939465a7037f5a",
    "declarations": [
      [
        1,
        "{\"schemaVersion\":1,\"minimumFreeBytes\":2147483648,\"progressTimeoutMs\":300000}"
      ]
    ],
    "anchorCount": 3
  },
  {
    "id": "home-archive-broker-response-lifetime",
    "path": "apps/kernel/src/slice/local_docker/broker.rs",
    "blob": "2a5ec0a4e08235e74bd5ac1f0ce36b1f73772772",
    "declarations": [
      [
        209,
        "fn configure_stream_deadlines() {}"
      ],
      [
        252,
        "fn execute_with_disk_evidence() {}"
      ]
    ],
    "anchorCount": 2,
    "sourceCommit": "3679fbb8bee37152fa694436297f558af2197d2c"
  },
  {
    "id": "shared-home-archive-disk-reserve-admission",
    "path": "apps/kernel/src/slice/local_docker/disk_admission.rs",
    "blob": "c97438de3b8ce2dd78d049f25e28be42a151a4a1",
    "declarations": [
      [
        137,
        "fn evaluate_slice_snapshot_disk_admission() {}"
      ],
      [
        157,
        "pub(super) fn with_slice_snapshot_disk_admission<T>() {}"
      ],
      [
        171,
        "pub(super) fn validate_slice_snapshot_disk_admission() {}"
      ]
    ],
    "anchorCount": 3
  },
  {
    "id": "shared-windows-pipe-producer-ownership",
    "path": "apps/kernel/src/io/windows_pipe_process.rs",
    "blob": "9ee7448312dbc063dbc5fb6cf37bf8299b8b9ac4",
    "declarations": [
      [
        59,
        "pub(crate) fn readiness() {}"
      ],
      [
        81,
        "pub(crate) struct Process {}"
      ],
      [
        88,
        "pub(crate) fn spawn() {}"
      ],
      [
        109,
        "pub(crate) fn stop() {}"
      ],
      [
        131,
        "fn resume() {}"
      ]
    ],
    "anchorCount": 5
  },
  {
    "id": "ordinary-backup-verification-cancellation-scope",
    "path": "apps/kernel/src/slice/local_docker/state.rs",
    "blob": "ebd3aebffc2efd0ecc8ed26d217bd51fee07d56d",
    "declarations": [
      [
        178,
        "pub fn validate_local_docker_slice_backup() {}"
      ],
      [
        975,
        "fn file_sha256() {}"
      ]
    ],
    "anchorCount": 2,
    "sourceCommit": "3679fbb8bee37152fa694436297f558af2197d2c"
  },
  {
    "id": "shared-provisioner-command-ownership",
    "path": "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
    "blob": "8b08b094f119638442dff57f0cce1235393c1358",
    "declarations": [
      [
        29,
        "docker() {}"
      ],
      [
        202,
        "run_guarded_command() {}"
      ],
      [
        229,
        "run_with_timeout() {}"
      ],
      [
        235,
        "run_with_file_stdin_timeout() {}"
      ],
      [
        276,
        "volume_inspect_reports_not_found() {}"
      ],
      [
        1596,
        "stop_container() {}"
      ],
      [
        1619,
        "destroy_container() {}"
      ]
    ],
    "anchorCount": 7,
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304"
  },
  {
    "id": "shared-saved-home-restore-stream-and-identity",
    "path": "apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh",
    "blob": "8b08b094f119638442dff57f0cce1235393c1358",
    "declarations": [
      [
        294,
        "saved_home_archive_identity() {}"
      ],
      [
        311,
        "restore_saved_home_volume() {}"
      ],
      [
        351,
        "prepare_home_volume() {}"
      ]
    ],
    "anchorCount": 3,
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304"
  },
  {
    "id": "broker-control-settlement",
    "path": "apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
    "blob": "ed76a2e5e296385a8917e26e0d266dd4863fe031",
    "declarations": [
      [
        911,
        "function spawnControl() {}"
      ],
      [
        921,
        "function handleIsMountpoint() {}"
      ],
      [
        929,
        "function unmountHandle() {}"
      ],
      [
        946,
        "function publishHandle() {}"
      ],
      [
        1248,
        "function inspectContainerMounts() {}"
      ],
      [
        1391,
        "function requireExactContainerMounts() {}"
      ]
    ],
    "anchorCount": 6,
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304"
  },
  {
    "id": "broker-build-duration-limit",
    "path": "apps/kernel/slice-linux-docker/managed-docker-broker.mjs",
    "blob": "ed76a2e5e296385a8917e26e0d266dd4863fe031",
    "declarations": [
      [
        1568,
        "function spawnBounded() {}"
      ]
    ],
    "anchorCount": 1,
    "sourceCommit": "3679fbb8bee37152fa694436297f558af2197d2c"
  },
  {
    "id": "shared-provisioner-command-guard",
    "path": "apps/kernel/slice-linux-docker/slice-command-guard.py",
    "blob": "a164518800a9a037d556975ea28a58636209d78e",
    "declarations": [
      [
        15,
        "def progress_timeout_seconds(): pass"
      ],
      [
        32,
        "def stop_orphaned_anchor(): pass"
      ],
      [
        41,
        "def finish_worker(): pass"
      ],
      [
        50,
        "def command_worker(): pass"
      ],
      [
        90,
        "def digest_worker(): pass"
      ],
      [
        115,
        "def source_digest_worker(): pass"
      ],
      [
        142,
        "def run_owned(): pass"
      ],
      [
        228,
        "def main(): pass"
      ]
    ],
    "anchorCount": 8,
    "sourceCommit": "7f882d06a9e10950708d9e7c6a8b20ec57356304"
  },
  {
    "id": "m20-owned-fallback-image-cleanup",
    "path": "apps/cli/scripts/lib/browser-state-drill-cleanup.mjs",
    "blob": "9c0f3f9fb2638c5b6314649c18d3e6c93c80f7d4",
    "declarations": [
      [
        1,
        "export function browserStateCleanupFailure() {}"
      ],
      [
        22,
        "export async function cleanupBrowserStateImages() {}"
      ]
    ],
    "anchorCount": 2
  },
  {
    "id": "m20-fallback-image-cleanup-callers",
    "path": "apps/cli/scripts/live-docker-slice-browser-state-drill.mjs",
    "blob": "d94d169c1a9003e6a6d77488f22aee43664915cf",
    "declarations": [
      [
        1025,
        "async function cleanup() {}"
      ]
    ],
    "anchorCount": 1
  }
];

for (const pinned of FINAL_ARCHIVE_DECLARATIONS) {
  test("final archive policy declaration retains review gap: " + pinned.id, () => {
    const rule = SOURCE_AUDIT_RULES.find(rule => rule.id === pinned.id);
    assert.ok(rule, "the inspected policy must be represented even without a lexical selector");
    assert.equal(rule.sourceCommit, pinned.sourceCommit ?? FINAL_ARCHIVE_SOURCE);
    assert.equal(rule.path, pinned.path);
    assert.equal(rule.blob, pinned.blob);
    const lines = Array.from({ length: Math.max(...pinned.declarations.map(([line]) => line)) }, () => "");
    for (const [line, declaration] of pinned.declarations) lines[line - 1] = declaration;
    withFixture({}, fixture => {
      fixture.addFile(pinned.path, lines.join("\n") + "\n", "100644", pinned.blob);
      const report = collect(fixture);
      const manual = report.entries.filter(entry => entry.candidateOrigin === "manual_source_rule" && entry.sourceClassification?.ruleId === pinned.id);
      assert.equal(manual.length, pinned.anchorCount);
      assert.ok(manual.every(entry => entry.sourceClassification.status === "source_inspected"));
      assert.ok(manual.every(entry => entry.semanticDisposition.status === "unreviewed"));
      assert.ok(manual.every(entry => entry.sourceClassification.independentDisposition === "pending"));
      assert.ok(!report.sourceAuditGaps.some(gap => gap.ruleId === pinned.id));
      if (pinned.id === "broker-build-duration-limit") {
        assert.ok(manual.every(entry => entry.sourceClassification.classification === "shared_owned_operation_lifetime"));
        assert.ok(manual.every(entry => entry.sourceClassification.openFindings.some(finding => finding.includes("independent changed-blob review"))));
      }
      assert.equal(report.status, "fail");
    });
    withFixture({}, fixture => {
      fixture.addFile(pinned.path, lines.join("\n") + "\n");
      const group = collect(fixture).sourceClassifications.find(group => group.ruleId === pinned.id);
      assert.equal(group?.status, "source_drift");
      assert.equal(group?.classification, null);
      assert.equal(group?.independentDisposition, "pending");
    });
  });
}


// MP-11: frozen independent conclusions may bind only the reviewed source.
test("MP-11 independent ledger covers each inspected scope without authorizing scope expansion", () => {
  assert.equal(new Set(INDEPENDENT_REVIEW_GROUPS.map(group => group.ruleId)).size, 134);
  assert.ok(INDEPENDENT_REVIEW_GROUPS.every(group => SOURCE_AUDIT_RULES.some(rule => rule.id === group.ruleId)));
  assert.equal(SOURCE_AUDIT_RULES.length, 147);
  assert.equal(new Set(DEFAULT_SEMANTIC_DISPOSITIONS.map(review => review.id)).size, DEFAULT_SEMANTIC_DISPOSITIONS.length);
  const first = DEFAULT_SEMANTIC_DISPOSITIONS[0];
  assert.equal(evaluateSemanticDisposition(first.anchor, { commit: first.sourceCommit, tree: first.sourceTree }, DEFAULT_SEMANTIC_DISPOSITIONS).status, "reviewed");
  for (const group of INDEPENDENT_REVIEW_GROUPS) {
    const rule = SOURCE_AUDIT_RULES.find(rule => rule.id === group.ruleId);
    assert.equal(group.path, rule.path);
    if (group.blob === rule.blob && group.sourceCommit === rule.sourceCommit) {
      assert.equal(group.sourceCommit, rule.sourceCommit);
    } else {
      // MP-08/MP-11 fixes do not inherit frozen independent approval.
      for (const anchor of group.anchors) {
        const changed = { path: rule.path, blob: rule.blob };
        assert.equal(evaluateSemanticDisposition(changed, { commit: rule.sourceCommit, tree: group.sourceTree }, DEFAULT_SEMANTIC_DISPOSITIONS).status, "unreviewed");
      }
    }
    assert.ok(group.anchors.length > 0);
  }
  for (const review of DEFAULT_SEMANTIC_DISPOSITIONS) {
    const source = { commit: review.sourceCommit, tree: review.sourceTree };
    const result = evaluateSemanticDisposition(review.anchor, source, [review]);
    assert.equal(result.disposition, review.disposition);
    assert.equal(result.gateEffect, review.disposition === "removal_required" ? "fail" : "reviewed");
    for (const field of ["path", "blob", "line", "column", "symbol", "category", "selector", "contextHash"]) {
      const value = review.anchor[field];
      const changed = typeof value === "number" ? value + 1 : `${value ?? ""}-changed`;
      assert.equal(evaluateSemanticDisposition({ ...review.anchor, [field]: changed }, source, [review]).status, "unreviewed", `${review.id}: ${field} drift`);
    }
    for (const field of ["commit", "tree"]) {
      assert.equal(evaluateSemanticDisposition(review.anchor, { ...source, [field]: "f".repeat(40) }, [review]).status, "unreviewed");
    }
  }
});

const MP11_NEW_RULE_EXCERPTS = {
  "broker-runtime-output-budget": [[45, "const MAX_OUTPUT_BYTES = 64 * 1024"], [1590, "async function execute(request) {"], [1685, "return spawnBounded(command, args, { env, maxBuffer: MAX_OUTPUT_BYTES }, false)"]],
  "placement-selected-slice-sandbox-policy": [[126, "pub fn from_config(config: &DaemonConfig) -> Self {"], [140, "allow_provider_sandbox_compatibility: managed_docker_broker_configured()"]],
  "broker-slice-resource-admission": [[439, "function validateProvisioner(action, environment, files) {"], [565, 'if (environment.CHARIOX_SLICE_DOCKER_CPUS) fail("CHARIOX_SLICE_DOCKER_CPUS is invalid")']],
  "placement-selected-kernel-dumpability": [[126, "pub fn initialize() {"], [150, "if configured && !make_process_nondumpable() {"], [215, "fn make_process_nondumpable() -> bool {"], [216, "libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0)"]],
  "broker-provisioner-environment-projection": [[325, "pub(super) fn provisioner_environment(command: &Command) -> BTreeMap<String, String> {"], [330, 'if !name.starts_with("CHARIOX_SLICE_") {'], [599, 'fn local_command(&self) -> Command {']],
};

for (const [ruleId, excerpt] of Object.entries(MP11_NEW_RULE_EXCERPTS)) {
  test(`MP-08/MP-11 new rule ${ruleId} retains candidates on blob drift without granting disposition`, () => {
    const rule = SOURCE_AUDIT_RULES.find(rule => rule.id === ruleId);
    const lines = Array.from({ length: Math.max(...excerpt.map(([line]) => line)) }, () => "");
    for (const [line, source] of excerpt) lines[line - 1] = source;
    const source = `${lines.join("\n")}\n`;
    let inspectedCount;
    for (const blob of [rule.blob, "e".repeat(40)]) {
      withFixture({}, fixture => {
        fixture.addFile(rule.path, source, "100644", blob);
        const candidates = collect(fixture).entries.filter(entry => entry.sourceClassification?.ruleId === ruleId);
        assert.ok(candidates.length > 0);
        assert.ok(candidates.every(entry => entry.sourceClassification.status === (blob === rule.blob ? "source_inspected" : "source_drift")));
        assert.ok(candidates.every(entry => entry.semanticDisposition.status === "unreviewed"), "fixture identity/context cannot reuse real-source reviews");
        for (const [, symbol] of rule.anchors) assert.ok(candidates.some(entry => entry.symbol === symbol && entry.candidateOrigin === "manual_source_rule"));
        if (blob === rule.blob) inspectedCount = candidates.length;
        else assert.equal(candidates.length, inspectedCount);
      });
    }
  });
}

for (const [id, declaration] of [
 ["shared-owned-broker-command", "export async function runBrokerCommand() {}"],
 ["ordinary-pinned-archive-verify", "pub(super) fn digest() {}"],
 ["common-slice-tuning-options", "pub(super) fn project() {}"],
]) test(`MP-08 MP-11 new common responsibility scope ${id} stays independently unreviewed`, () => {
 const rule=SOURCE_AUDIT_RULES.find(rule=>rule.id===id)
 assert.ok(rule,"new runtime responsibility modules must retain an audit scope")
 const source=declaration+"\n"
 for(const blob of [rule.blob,"e".repeat(40)]) withFixture({}, fixture=>{
  fixture.addFile(rule.path,source,"100644",blob)
  const report=collect(fixture)
  const candidates=report.entries.filter(entry=>entry.sourceClassification?.ruleId===id)
  assert(candidates.length>0)
  assert(candidates.every(entry=>entry.semanticDisposition.status==="unreviewed"))
  assert(candidates.every(entry=>entry.sourceClassification.status===(blob===rule.blob?"source_inspected":"source_drift")))
  assert.equal(report.status,"fail")
 })
})

// MP-11: reviewed high-risk triage scopes must retain candidates on drift.
const MP11B_SCOPE_FIXTURES = [
  {"id": "mp11b-ready-machine-kernel-projection", "path": "apps/web/src/ui/waiting-room-runtime-placement.ts", "blob": "5eb52b577ba4a3f85925e1f543b4b339167438b0", "excerpt": [[47, "export function waitingRoomMachineOptions("], [103, "export function waitingRoomKernelsForSelectedMachine("]], "outsideLine": 134},
  {"id": "mp11b-ready-machine-home-readiness", "path": "apps/web/src/ui/waiting-room-launch-readiness.ts", "blob": "e3ad8eb2976de03c725bdc2bd6a33b2479aa476f", "excerpt": [[108, "      return managedEnvironmentLaunchReadiness(`Connect to ${managedEnvironment.name} and start session`)"]], "outsideLine": 115},
  {"id": "mp11b-shared-host-unit", "path": "deploy/managed-kernel/chariox-managed-bootstrap.service", "blob": "902948442e4ecc771b86e56be1cfa79e83ea583e", "excerpt": [[26, "Environment=CHARIOX_MANAGED_BOOTSTRAP_PATH=/var/lib/chariox/managed-bootstrap.json"]], "outsideLine": 63},
  {"id": "mp11b-broker-service", "path": "deploy/managed-kernel/chariox-slice-broker.service", "blob": "8195754884b2c17609ce7812b914c49b8935c6a3", "excerpt": [[32, "ReadWritePaths=/var/lib/chariox-docker /var/lib/chariox-slice-disk-quota /var/lib/chariox-slice-share /run/chariox-docker"]], "outsideLine": 41},
  {"id": "mp11b-rootless-lifecycle-service", "path": "deploy/managed-kernel/chariox-rootless-docker.service", "blob": "8aae4b3f4809d3bc3d165fdaa137480927edee3b", "excerpt": [[30, "ProtectSystem=strict"]], "outsideLine": 46},
  {"id": "mp11b-rootless-engine-service", "path": "apps/kernel/slice-linux-docker/chariox-rootless-engine.service", "blob": "4a146d543dd669d493cb4e446a4269c7939c3c69", "excerpt": [[17, "UMask=0077"]], "outsideLine": 22},
  {"id": "mp11b-quota-allocator-service", "path": "apps/kernel/slice-linux-docker/chariox-slice-disk-quota-allocator.service", "blob": "8e6bee212317d8beea536916e12002ddf7df934c", "excerpt": [[27, "ProtectHome=true"]], "outsideLine": 44},
  {"id": "mp11b-data-volume-admission-service", "path": "apps/kernel/slice-linux-docker/chariox-data-volume-admission.service", "blob": "cfadfdfca462e4752ea7a69d90351227da145881", "excerpt": [[20, "ProtectSystem=strict"]], "outsideLine": 35},
  {"id": "mp11b-image-builder-cleanup-service", "path": "deploy/managed-kernel/chariox-image-builder-cleanup@.service", "blob": "cf4006dbd65c3a6b4a25c2c543f9b390ed1aa478", "excerpt": [[17, "PrivateTmp=true"]], "outsideLine": 24},
  {"id": "mp11b-scm-home-selection", "path": "apps/kernel/src/managed_context/scm.rs", "blob": "ac4c45f2afb93daefd8749f9f06cdea4642e7562", "excerpt": [[113, "    pub(crate) fn source_from_process() -> Result<Self, DaemonError> {"], [139, "    pub(crate) fn managed_target(home: PathBuf) -> Result<Self, DaemonError> {"]], "outsideLine": 174},
  {"id": "mp11b-pty-process-marker", "path": "apps/kernel/src/pty/manager.rs", "blob": "1560ca2dcbc15dae6c0240c477233eb67b8b2867", "excerpt": [[271, "            \"CHARIOX_MANAGED_PROVIDER_PROCESS\".to_string(),"]], "outsideLine": 291},
  {"id": "mp11b-opencode-discovery-namespace-adapter", "path": "apps/kernel/src/provider/opencode/discovery.rs", "blob": "a466ebb02f58b10e08af75464248c86dd792e983", "excerpt": [[19, "    pub(crate) fn apply_to_launch_args("]], "outsideLine": 96},
  {"id": "mp11b-preparation-namespace-adapters", "path": "apps/kernel/src/provider/managed_isolation.rs", "blob": "37ae2123aaea0e9eaf82374a3ef45018a69cc017", "excerpt": [[213, "pub(crate) fn apply_preparation_home_to_managed_launch("], [293, "pub(crate) fn managed_launch_has_preparation_home(args: &[String], host_home: &Path) -> bool {"]], "outsideLine": 319},
  {"id": "mp11b-provider-reported-path-adapter", "path": "apps/kernel/src/provider/managed_isolation.rs", "blob": "37ae2123aaea0e9eaf82374a3ef45018a69cc017", "excerpt": [[458, "pub(crate) fn provider_reported_path_on_kernel("]], "outsideLine": 518},
  {"id": "mp11b-runtime-environment-inputs", "path": "apps/kernel/src/config/env_loader.rs", "blob": "b7707033b9d9fe03f3ad911ddac38e6f119bc5ec", "excerpt": [[188, "                \"CHARIOX_MANAGED_SLICE_RELAY_OWNER_PUBLIC_KEY\","]], "outsideLine": 210},
  {"id": "mp11b-project-validation-control-scrub", "path": "apps/kernel/src/runtime/state/project_environment_setup_validation.rs", "blob": "59708026b354c26305d248016dedd1ca47f7fd03", "excerpt": [[489, "pub(super) fn worker_validation_environment_with_home_and_definition("]], "outsideLine": 537},
  {"id": "mp11b-installer-exact-control-repair", "path": "deploy/managed-kernel/install-image.sh", "blob": "c3d9d546c6a5d771581a0094338eaf9ab0efea34", "excerpt": [[232, "repair_root_control_file() {"], [246, "repair_root_control_tree() {"], [272, "repair_root_control_state() {"]], "outsideLine": 284},
  {"id": "mp11b-fresh-rebuild-process-negative-guards", "path": "apps/kernel/src/managed_bootstrap/freshness.rs", "blob": "d9d5c11b1b9189cbd4fbd2826a2845bfe04e255c", "excerpt": [[510, "pub(super) fn validate_process_observations("], [573, "fn is_bubblewrap(observation: &ProcessObservation) -> bool {"]], "outsideLine": 588},
  {"id": "mp11b-inner-provider-bwrap-launcher", "path": "apps/kernel/slice-linux-docker/docker/managed-provider-bwrap.sh", "blob": "88741c2c0ebb04f8c2c8ab75a884b87e77b444bc", "excerpt": [[12, "bwrap=(/usr/bin/bwrap)"]], "outsideLine": 22},
  {"id": "mp11b-slice-runtime-provider-home", "path": "apps/kernel/slice-linux-docker/docker/start-runtime.sh", "blob": "195ce5230578b6ed93068918d0d7f9ce9227c73e", "excerpt": [[33, "PROVIDER_HOME=\"${CHARIOX_MANAGED_PROVIDER_HOME:-$HOME/.chariox/provider-home}\""]], "outsideLine": 47},
  {"id": "mp11b-image-topology-admission", "path": "deploy/managed-kernel/prepare-hetzner-image.sh", "blob": "f9c8f12c97fdb7e66df523615e4bf7f30daf8ec8", "excerpt": [[227, "  '') fail \"CHARIOX_MANAGED_PROVIDER_TOPOLOGY must be explicitly set to path1 or shared_host\" ;;"]], "outsideLine": 234},
  {"id": "mp11b-bootstrap-api-configuration", "path": "apps/api/src/managed-environments/bootstrap-config.ts", "blob": "ebed439b48604dbbea6e0ea539af524ecee1b57c", "excerpt": [[1, "export function managedKernelCloudApiUrl(env: NodeJS.ProcessEnv): string {"]], "outsideLine": 13},
  {"id": "mp11b-placement-catalog-environment", "path": "apps/api/src/managed-environments/placement-catalog.ts", "blob": "0bb1a248af873f0739090dd20524136c0dc6b667", "excerpt": [[25, "export function configuredManagedEnvironmentPlacementCatalog("], [120, "function parsePlacementSelections("]], "outsideLine": 212},
  {"id": "mp11b-candidate-placement-environment", "path": "apps/api/src/managed-environments/configured-operator-candidate-admission.ts", "blob": "bacffca9218f533a1139c6a0b1ea8610b82e1aa2", "excerpt": [[13, "export function configuredOperatorCandidateAdmission(env: NodeJS.ProcessEnv) {"]], "outsideLine": 56},
  {"id": "mp11b-quiescence-release-approval", "path": "apps/api/src/managed-environments/auto-stop-quiescence-release-policy.ts", "blob": "f425167d5b1ddbcaed35b8283d6e750aac616c8f", "excerpt": [[6, "export function configuredQuiescenceReleaseDigests(env: Readonly<Record<string, string | undefined>>): readonly string[] {"], [19, "export function requireQuiescenceRelease(digest: string | null, approved: readonly string[]): void {"]], "outsideLine": 28},
  {"id": "mp11b-worker-placement-lifetime-policy", "path": "apps/api/src/disposable-workers/placement-policy.ts", "blob": "4aa94f2b02cf027beff46a1981a701866eb6c393", "excerpt": [[9, "export function configuredDisposableWorkerPlacementPolicy(env: NodeJS.ProcessEnv): DisposableWorkerPlacementPolicy {"]], "outsideLine": 84},
  {"id": "mp11b-activity-registration-selection", "path": "apps/kernel/src/runtime/managed_kernel_activity.rs", "blob": "c194aff6b1c348d518f9e4854bc22defa1abcbd1", "excerpt": [[74, "    pub(crate) fn from_runtime("]], "outsideLine": 127},
  {"id": "mp11b-publication-route-path-filter", "path": "apps/server/src/publication-agent-app-effects.ts", "blob": "901d042896a86a189454443e4c1bda057f6c3622", "excerpt": [[168, "function routeAllowsOverlay(route: AgentAppRouteConfig | undefined, path: string): boolean {"]], "outsideLine": 201},
  {"id": "mp11b-publication-route-filter-schema", "path": "apps/server/src/publication-agent-app-schema.ts", "blob": "ff5b4032af14e220d34bab413921f4d416a98ff9", "excerpt": [[100, "function validateRoute(route: AgentAppRouteConfig, actionIds: Set<string>): void {"]], "outsideLine": 140},
  {"id": "mp11b-release-archive-service-mount", "path": "deploy/openship/cloud-release.compose.yml", "blob": "bcb3933e17543c24410c9710f7f9ac3d890a5cf7", "excerpt": [[67, "      - chariox-managed-releases:/var/lib/chariox-managed-releases:ro"]], "outsideLine": 72},
  {"id": "mp11b-shared-live-sync-mode-projection", "path": "packages/kernel-client/src/workspace-live-sync-mode.ts", "blob": "d19e756e1c7c1aafa1d8815ea94674dbb47cd60d", "excerpt": [[11, "export function parseWorkspaceLiveSyncModeCommand(value: string): WorkspaceLiveSyncModeCommandInput | null {"], [22, "export function formatWorkspaceLiveSyncModeLabel(mode: WorkspaceLiveSyncModeLabelInput): string {"]], "outsideLine": 74},
  {"id": "mp11b-web-live-sync-mode-projection", "path": "apps/web/src/terminal/workspace-live-sync-mode.ts", "blob": "5e125bd795151913a7baaae4dd29749fdc44e77c", "excerpt": [[7, "export function normalizeWorkspaceLiveSyncMode(value: unknown): WorkspaceLiveSyncMode | null {"]], "outsideLine": 51},
  {"id": "mp11b-ios-live-sync-mode-command", "path": "apps/ios/CharioxPackage/Sources/CharioxFeature/State/CharioxAppModelCommands.swift", "blob": "76084e6866b37607d0c30b2122e84597c1821ea6", "excerpt": [[157, "    case \"managed\", \"tracked\":"]], "outsideLine": 180},
  {"id": "mp11b-runtime-projection-fixture", "path": "packages/kernel-client/src/session-runtime-projection.test-support.ts", "blob": "e196b7a3d2fe1ca8af77ce037f667fc3c0f93922", "excerpt": [[17, "export function workspaceLiveSyncStatus("]], "outsideLine": 37},
  {"id": "mp11b-soak-sandbox-root-guard", "path": "apps/cli/scripts/lib/browser-computer-soak-runtime.mjs", "blob": "d2041162b4ed01208f69dd2e91cbd888ec093a54", "excerpt": [[853, "export function assertSandboxCapableChromiumIdentity({"]], "outsideLine": 867},
  {"id": "mp11b-ready-machine-forced-preparation", "path": "apps/cli/src/waiting-room-controller.ts", "blob": "be39b058449d1d81ff7ff48a95b19d92dba20a23", "excerpt": [[396, "function waitingRoomManagedLaunchSelection("]], "outsideLine": 429},
  {"id": "mp11b-ready-machine-transfer-workspace-reset", "path": "apps/cli/src/waiting-room-managed-environment-launch-controller.ts", "blob": "d82059e8aa84243145387136fbb4375672655704", "excerpt": [[313, "function managedProjectPreparation("], [264, "          const workspacePath = primaryWorkspacePath(launchTarget)"], [323, "  const workspacePath = primaryWorkspacePath(launchTarget)"]], "outsideLine": 357},
  {"id": "mp11b-ready-machine-launch-choice-reset", "path": "apps/cli/src/cli-waiting-room-composition.ts", "blob": "78ff8a8a8e119e7082d60bed642c5cc49c87146a", "excerpt": [[558, "  const prepareManagedSessionLaunch = async ("]], "outsideLine": 654},
  {"id": "mp11b-ready-machine-execution-worker-reset", "path": "apps/web/src/terminal/waiting-room-managed-environment-launch-controller.ts", "blob": "05f6edbb5c5cb293ebf45b5ffc15f560ea6b5c41", "excerpt": [[384, "    const readyState = updateWaitingRoomKernelData(this.deps.state(), {"]], "outsideLine": 425}
];

for (const pinned of MP11B_SCOPE_FIXTURES) {
  test(`MP-11 out-of-scope review retains fail-closed fixture and drift: ${pinned.id}`, () => {
    const rule = SOURCE_AUDIT_RULES.find(rule => rule.id === pinned.id);
    assert.ok(rule, "MP-11 reviewed branch needs an exact scope");
    assert.equal(rule.path, pinned.path);
    assert.equal(rule.blob, pinned.blob);
    const lines = Array.from({ length: pinned.outsideLine }, () => "");
    for (const [line, excerpt] of pinned.excerpt) lines[line - 1] = excerpt;
    lines[pinned.outsideLine - 1] = 'const outside = "CHARIOX_MANAGED_MP11B_UNINSPECTED";';
    let inspectedCount;
    for (const blob of [pinned.blob, "e".repeat(40)]) {
      withFixture({}, fixture => {
        fixture.addFile(pinned.path, `${lines.join("\n")}\n`, "100644", blob);
        const report = collect(fixture);
        const candidates = report.entries.filter(entry => entry.sourceClassification?.ruleId === pinned.id);
        assert.ok(candidates.length > 0);
        assert.ok(candidates.every(entry => entry.sourceClassification.status === (blob === pinned.blob ? "source_inspected" : "source_drift")));
        assert.ok(candidates.every(entry => entry.semanticDisposition.status === "unreviewed"), "fixture source/context cannot reuse frozen real-source approvals");
        assert.ok(candidates.every(entry => entry.sourceClassification.independentDisposition === "pending"));
        for (const [, symbol] of rule.anchors) assert.ok(candidates.some(entry => entry.candidateOrigin === "manual_source_rule" && entry.symbol === symbol));
        const outside = report.entries.filter(entry => entry.line === pinned.outsideLine && entry.path === pinned.path);
        assert.ok(outside.length > 0);
        assert.ok(outside.every(entry => entry.sourceClassification?.ruleId !== pinned.id && entry.semanticDisposition.status === "unreviewed"));
        if (blob === pinned.blob) inspectedCount = candidates.length;
        else assert.equal(candidates.length, inspectedCount);
        assert.equal(report.status, "fail");
      });
    }
  });
}

// MP-08/MP-11: new corrected seams remain provisional and fail closed on drift.
for (const id of ["parity3-supervisor-broker-response-owner", "parity3-interactive-slice-protocol-admission", "parity3-interactive-slice-create-admission"]) {
  test(`MP-08/MP-11 corrected parity3 scope retains unreviewed anchors on drift: ${id}`, () => {
    const rule = SOURCE_AUDIT_RULES.find(rule => rule.id === id);
    assert.ok(rule);
    const lines = Array.from({ length: Math.max(...rule.ranges.map(([, end]) => end)) }, () => "");
    rule.anchors.forEach(([, symbol], index) => { lines[rule.ranges[0][0] + index - 1] = `function ${symbol}() {}`; });
    for (const blob of [rule.blob, "e".repeat(40)]) withFixture({}, fixture => {
      fixture.addFile(rule.path, lines.join("\n") + "\n", "100644", blob);
      const report = collect(fixture);
      const candidates = report.entries.filter(entry => entry.sourceClassification?.ruleId === id);
      assert.ok(candidates.length >= rule.anchors.length);
      assert.ok(candidates.every(entry => entry.sourceClassification.status === (blob === rule.blob ? "source_inspected" : "source_drift")));
      assert.ok(candidates.every(entry => entry.semanticDisposition.status === "unreviewed"));
      assert.equal(report.status, "fail");
    });
  });
}

// MP-01/MP-04/MP-05/MP-06/MP-07/MP-09/MP-11: implementation observations
// stay pending on both exact inspected blobs and source drift.
for (const id of ["impld-path1-role-service-policy", "impld-repository-component-safety",
  "impld-repository-final-destination", "impld-auto-stop-outage-policy",
  "impld-auto-stop-operation", "impld-auto-stop-reservation"]) {
  test(`MP-11 release D scope ${id} stays pending and detects source drift`, () => {
    const rule = SOURCE_AUDIT_RULES.find((candidate) => candidate.id === id)
    assert.ok(rule, `missing release D scope ${id}`)
    for (const status of ["source_inspected", "source_drift"]) withFixture({}, (fixture) => {
      const source = "\n".repeat((rule.ranges?.[0]?.[0] ?? 1) - 1)
        + rule.anchors.map(([, symbol]) => `function ${symbol}() {}`).join("\n") + "\n"
      fixture.addFile(rule.path, source, "100644", status === "source_inspected" ? rule.blob : "f".repeat(40))
      const report = collect(fixture)
      const scoped = report.entries.filter((entry) => entry.sourceClassification?.ruleId === id)
      assert.ok(scoped.length > 0)
      assert.ok(scoped.every((entry) => entry.sourceClassification.status === status
        && entry.semanticDisposition.status === "unreviewed"))
    })
  })
}

test("MP-11 release D Cloud scopes are not missing OSS source declarations", () => {
  withFixture({}, (fixture) => {
    const report = collect(fixture)
    assert.ok(!report.sourceAuditGaps.some((gap) => gap.ruleId.startsWith("impld-auto-stop-")))
  })
})

test("MP-02/MP-08/MP-11 enrolled launch adapter observation stays independently unreviewed", () => {
  const rule = SOURCE_AUDIT_RULES.find((rule) => rule.id === "enrolled-machine-common-launch-adapter");
  assert.ok(rule);
  for (const blob of [rule.blob, "e".repeat(40)]) {
    withFixture({}, (fixture) => {
      fixture.addFile(rule.path, "export function prepareWaitingRoomEnrolledLaunch(options) { return options; }\n", "100644", blob);
      const report = collect(fixture);
      const candidate = report.entries.find((entry) => entry.sourceClassification?.ruleId === rule.id);
      assert.ok(candidate);
      assert.equal(candidate.semanticDisposition.status, "unreviewed");
      assert.equal(candidate.sourceClassification.authoritative, false);
      assert.equal(candidate.sourceClassification.independentDisposition, "pending");
      assert.equal(candidate.sourceClassification.classification, blob === rule.blob
        ? "shared_enrolled_machine_launch_adapter" : null);
    });
  }
});


test("MP-02/MP-08/MP-11 enrolled Cloud launch scopes do not require missing OSS files", () => {
  const rules = SOURCE_AUDIT_RULES.filter(rule => rule.repository === "cloud");
  assert.ok(rules.some(rule => rule.id === "mp11b-ready-machine-execution-worker-reset"));
  withFixture({}, fixture => {
    const report = collect(fixture);
    assert.ok(!report.sourceAuditGaps.some(gap => rules.some(rule => rule.id === gap.ruleId)));
  });
});

// MP-11: exact exclusions must not hide a future executable sibling.
test("MP-11 native source/config/installer formats and exact fixture exclusions", () => {
  for (const [path, format] of [
    ["apps/app-worker/src/worker.cc", "c"], ["apps/app-worker/src/record.h", "c"],
    ["apps/app-storage-helper/src/permission.rs", "rust"],
    ["deploy/local-macos/dev.example.plist", "config"], ["scripts/macos-pkg/postinstall", "shell"],
  ]) assert.equal(classifyProductionPath(path), format);
  for (const path of ["archive/0", "signed/0", "signed/1", "signed/2", "signed/3"]) {
    assert.equal(classifyProductionPath("packages/app-package/fuzz/seeds/" + path), null);
  }
  assert.equal(classifyProductionPath("packages/app-package/fuzz/seeds/signed/execution.rs"), "rust");
  assert.equal(classifyProductionPath("apps/kernel/src/runtime/state/testdata/claude-setup-token-2.1.281-start.pty"), null);
  for (const path of ["packages/app-package/fuzz/seeds/signed/4", "apps/kernel/src/runtime/state/testdata/other.pty",
    "apps/app-worker/src/unknown.selector", "apps/app-storage-helper/src/unknown.selector", "scripts/macos-pkg/preinstall"]) {
    assert.throws(() => classifyProductionPath(path), /unclassified production/);
  }
  withFixture({}, (fixture) => {
    for (const [path, text] of [
      ["apps/app-worker/src/control.cc", 'const char *control = "CHARIOX_MANAGED_NATIVE";'],
      ["apps/app-worker/src/control.h", '#define CONTROL "CHARIOX_MANAGED_HEADER"'],
      ["deploy/local-macos/control.plist", '<string>CHARIOX_MANAGED_PLIST</string>'],
      ["scripts/macos-pkg/postinstall", 'control="$CHARIOX_MANAGED_INSTALLER"'],
      ["apps/app-storage-helper/src/control.rs", 'let control = "CHARIOX_MANAGED_STORAGE";'],
    ]) fixture.addFile(path, text + "\n");
    const report = collect(fixture);
    for (const selector of ["NATIVE", "HEADER", "PLIST", "INSTALLER", "STORAGE"]) {
      const entry = report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_" + selector);
      assert.ok(entry, selector);
      assert.equal(entry.semanticDisposition.status, "unreviewed");
    }
    assert.equal(report.status, "fail");
  });
});

test("MP-11 binary source is rejected rather than silently omitted", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("apps/app-worker/src/control.cc", Buffer.from([0, 65, 66]));
    assert.throws(() => collect(fixture), /unsupported binary source/);
  });
});

test("MP-11 BOM fixture decoding retains the first-line physical column", () => {
  withFixture({}, (fixture) => {
    const path = "apps/cli/src/bom-control.ts";
    const selector = "CHARIOX_MANAGED_BOM_FIXTURE";
    const text = '\ufeffconst control = "' + selector + '";\r\n';
    fixture.addFile(path, text);
    const report = collect(fixture);
    const entry = report.entries.find((entry) => entry.selector === selector);
    assert.equal(entry?.line, 1);
    assert.equal(entry?.column, text.indexOf(selector) + 1);
    assert.equal(entry?.contextHash, createHash("sha256").update(text.trim()).digest("hex"));
    assert.equal(entry?.semanticDisposition.status, "unreviewed");
    assert.equal(report.status, "fail");
    fixture.addFile(path, Buffer.from([0xef, 0xbb, 0xbf, 0xc3, 0x28]));
    assert.throws(() => collect(fixture), { code: "ERR_ENCODING_INVALID_ENCODED_DATA" });
  });
});

test("MP-11 BOM Git blob decoding retains exact identity and first-line physical column", () => {
  const root = mkdtempSync(join(tmpdir(), "chariox-mp11-bom-"));
  const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" });
  try {
    git("init", "--quiet");
    mkdirSync(join(root, "apps/cli/src"), { recursive: true });
    const path = "apps/cli/src/bom-control.ts";
    const selector = "CHARIOX_MANAGED_BOM_GIT";
    const text = '\ufeffconst control = "' + selector + '";\r\n';
    const commit = () => {
      git("add", path);
      git("-c", "user.name=MP-11 fixture", "-c", "user.email=fixture@example.invalid", "commit", "--quiet", "-m", "MP-11 BOM fixture");
      return git("rev-parse", "HEAD").trim();
    };
    writeFileSync(join(root, path), text);
    const sourceCommit = commit();
    const blob = git("rev-parse", "HEAD:" + path).trim();
    const report = collectSourceInventory({ sourceRoot: root, expectedCommit: sourceCommit });
    const entry = report.entries.find((entry) => entry.selector === selector);
    assert.equal(report.source.commit, sourceCommit);
    assert.equal(entry?.blob, blob);
    assert.equal(entry?.line, 1);
    assert.equal(entry?.column, text.indexOf(selector) + 1);
    assert.equal(entry?.contextHash, createHash("sha256").update(text.trim()).digest("hex"));
    assert.equal(entry?.semanticDisposition.status, "unreviewed");
    assert.equal(report.status, "fail");
    writeFileSync(join(root, path), Buffer.from([0xef, 0xbb, 0xbf, 0xc3, 0x28]));
    const malformedCommit = commit();
    assert.throws(() => collectSourceInventory({ sourceRoot: root, expectedCommit: malformedCommit }),
      { code: "ERR_ENCODING_INVALID_ENCODED_DATA" });
  } finally { rmSync(root, { recursive: true, force: true }); }
});

for (const suffix of [".tsfrag", ".mjsfrag"]) {
  test("MP-11 BOM at an assembled " + suffix + " boundary retains physical and assembled columns", () => {
    withFixture({}, (fixture) => {
      const selector = "CHARIOX_MANAGED_BOM_FRAGMENT";
      const fragments = ['const selector = "', '\ufeff' + selector + '";\nselector;'];
      assert.equal(runInNewContext(fragments.join("")), "\ufeff" + selector);
      const directory = fragmentFixture(fixture, suffix, fragments);
      const report = collect(fixture);
      const entry = report.entries.find((entry) => entry.selector === selector);
      assert.equal(entry?.path, directory + "/part-002" + suffix);
      assert.equal(entry?.line, 1);
      assert.equal(entry?.column, 2);
      assert.equal(entry?.fragmentSource.assembledColumn, fragments[0].length + 2);
      assert.equal(entry?.fragmentSource.matchSegments[0].column, 2);
      assert.equal(entry?.semanticDisposition.status, "unreviewed");
      assert.equal(report.status, "fail");
    });
  });
}

test("MP-11 standard unified sections retain exact physical lines and old/new paths", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("deploy/native-controls.patch", [
      "--- a/apps/app-worker/src/old.c\t2026-10-04", "+++ b/apps/app-worker/src/new.cc\t2026-10-04", "@@ -1 +1 @@",
      '-const char *old = "CHARIOX_MANAGED_OLD";', '+const char *next = "CHARIOX_MANAGED_NEXT";',
      "--- /dev/null", "+++ apps/app-worker/src/two.h", "@@ -0,0 +1 @@",
      '+const char *next = "CHARIOX_MANAGED_SECOND";',
      "--- apps/app-worker/src/deleted.cc", "+++ /dev/null", "@@ -2 +0,0 @@",
      '-const char *old = "CHARIOX_MANAGED_DELETED";',
    ].join("\n") + "\n");
    const report = collect(fixture);
    const entry = (selector) => report.entries.find((entry) => entry.selector === "CHARIOX_MANAGED_" + selector);
    assert.equal(entry("OLD").line, 4);
    assert.equal(entry("OLD").patchSource.path, "apps/app-worker/src/old.c");
    assert.equal(entry("OLD").patchSource.change, "removed");
    assert.equal(entry("OLD").sourceRoleHints.executionRole, "removed_patch_source_candidate");
    assert.equal(entry("NEXT").line, 5);
    assert.equal(entry("NEXT").patchSource.path, "apps/app-worker/src/new.cc");
    assert.equal(entry("SECOND").line, 9);
    assert.equal(entry("SECOND").patchSource.path, "apps/app-worker/src/two.h");
    assert.equal(entry("DELETED").line, 13);
    assert.equal(entry("DELETED").patchSource.oldLine, 2);
    assert.ok(["OLD", "NEXT", "SECOND", "DELETED"].every((selector) => entry(selector).semanticDisposition.status === "unreviewed"));
  });
});

for (const path of ["../escape.ts", "foo/../escape.ts", "/absolute.ts", "foo//control.ts", "./control.ts", "foo\\control.ts", "C:/control.ts"]) {
  test("MP-11 unsafe standard patch path rejected: " + path, () => {
    withFixture({}, (fixture) => {
      fixture.addFile("deploy/unsafe.patch", ["--- a/" + path, "+++ b/apps/api/src/control.ts", "@@ -1 +1 @@", "-old", "+new"].join("\n"));
      assert.throws(() => collect(fixture), /unsafe patch source path/);
    });
  });
}
for (const [name, lines] of [
  ["missing new header", ["--- a/apps/api/src/control.ts", "@@ -1 +1 @@", "-old", "+new"]],
  ["unsafe Git header", ["diff --git a/../control.ts b/apps/api/src/control.ts", "@@ -1 +1 @@", "-old", "+new"]],
  ["mismatching Git header", ["diff --git a/apps/api/src/control.ts b/apps/api/src/control.ts", "--- a/apps/api/src/other.ts", "+++ b/apps/api/src/control.ts", "@@ -1 +1 @@", "-old", "+new"]],
  ["count underflow", ["--- a/apps/api/src/control.ts", "+++ b/apps/api/src/control.ts", "@@ -1 +1 @@", "-old", "-extra", "+new"]],
  ["empty section", ["--- a/apps/api/src/control.ts", "+++ b/apps/api/src/control.ts"]],
  ["unknown old format", ["--- a/apps/api/src/control.unknown", "+++ b/apps/api/src/control.ts", "@@ -1 +1 @@", "-old", "+new"]],
  ["unmatched new header", ["+++ b/apps/api/src/control.ts", "@@ -1 +1 @@", "-old", "+new"]],
]) test("MP-11 malformed/unsupported patch rejects " + name, () => {
  withFixture({}, (fixture) => {
    fixture.addFile("deploy/invalid.patch", lines.join("\n") + "\n");
    assert.throws(() => collect(fixture), /patch|unclassified production/);
  });
});


test("MP-11 every current declaration binds exact Git identity/range/physical hash", () => {
  for (const record of CURRENT_DECLARATION_EXPECTATIONS) {
    // The Cloud expectation is checked in the paired exact-source scan. This
    // OSS repository intentionally does not assume Cloud objects are present.
    if (record.path.startsWith("apps/web/")) continue;
    const root = new URL("../../..", import.meta.url).pathname;
    const blob = execFileSync("git", ["rev-parse", record.sourceCommit + ":" + record.path], { cwd: root, encoding: "utf8" }).trim();
    assert.equal(blob, record.blob);
    assert.equal(execFileSync("git", ["rev-parse", record.sourceCommit + "^{tree}"], { cwd: root, encoding: "utf8" }).trim(), record.sourceTree);
    const text = execFileSync("git", ["cat-file", "blob", blob], { cwd: root, encoding: "utf8" });
    const source = { commit: record.sourceCommit, tree: record.sourceTree };
    const file = { path: record.path, blob, text };
    for (const anchor of record.declarations) {
      assert.ok(record.range.some(([start, end]) => start === anchor.line && end === anchor.line));
      assert.equal(createHash("sha256").update(text.split(/\r?\n/)[anchor.line - 1].trim()).digest("hex"), anchor.contextHash);
      assert.ok(currentDeclarationCandidates(file, source).some((candidate) => candidate.ruleId === record.ruleId
        && candidate.symbol === record.symbol && candidate.lineIndex + 1 === anchor.line));
    }
    assert.equal(record.semanticApproval, false);
    assert.equal(currentDeclarationCandidates({ ...file, blob: "a".repeat(40) }, source).length, 0);
    assert.equal(currentDeclarationCandidates(file, { ...source, commit: "a".repeat(40) }).length, 0);
    assert.equal(currentDeclarationCandidates(file, { ...source, tree: "a".repeat(40) }).length, 0);
    const renamed = text.replaceAll(record.symbol, "changed_" + record.symbol);
    assert.ok(!currentDeclarationCandidates({ ...file, text: renamed }, source).some((candidate) => candidate.symbol === record.symbol));
  }
});

test("MP-11 new native/config formats retain selectors after literal/hash boundaries", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("apps/app-worker/src/raw.cc", 'const char *raw = R"(\" // CHARIOX_MANAGED_RAW)";\n');
    fixture.addFile("deploy/local-macos/raw.plist", '<string>#CHARIOX_MANAGED_XML</string>\n');
    const report = collect(fixture);
    for (const selector of ["CHARIOX_MANAGED_RAW", "CHARIOX_MANAGED_XML"]) {
      const entry = report.entries.find((entry) => entry.selector === selector);
      assert.ok(entry);
      assert.equal(entry.semanticDisposition.status, "unreviewed");
    }
  });
});


test("MP-11 current bounded reviews do not repin the historical review module", () => {
  const root = new URL("../../..", import.meta.url).pathname;
  const path = "apps/cli/scripts/lib/managed-parity-semantic-reviews.mjs";
  const original = execFileSync("git", ["show", "9334141d420f8a32393f206102c5b8b4a1b0b609:" + path], { cwd: root });
  assert.deepEqual(readFileSync(join(root, path)), original);
  for (const review of CURRENT_SEMANTIC_REVIEWS) {
    assert.ok(review.independentReview.rationale.startsWith("MP-"));
    assert.ok(review.independentReview.reviewer.includes("scanner implementer"));
    assert.ok(!INDEPENDENT_REVIEW_GROUPS.some((group) => group.sourceCommit === review.sourceCommit));
    assert.equal(evaluateSemanticDisposition(review.anchor, { commit: review.sourceCommit, tree: review.sourceTree }, [review]).status, "reviewed");
    assert.equal(evaluateSemanticDisposition({ ...review.anchor, contextHash: "f".repeat(64) }, { commit: review.sourceCommit, tree: review.sourceTree }, [review]).status, "unreviewed");
    if (review.anchor.path.startsWith("apps/web/")) continue;
    const blob = execFileSync("git", ["rev-parse", review.sourceCommit + ":" + review.anchor.path], { cwd: root, encoding: "utf8" }).trim();
    assert.equal(blob, review.anchor.blob);
    const line = execFileSync("git", ["cat-file", "blob", blob], { cwd: root, encoding: "utf8" }).split(/\r?\n/)[review.anchor.line - 1];
    assert.equal(createHash("sha256").update(line.trim()).digest("hex"), review.anchor.contextHash);
    const group = CURRENT_REVIEW_GROUPS.find((group) => group.path === review.anchor.path && group.anchors.some((anchor) => anchor[0] === review.anchor.line && anchor[1] === review.anchor.column && anchor[3] === review.anchor.category));
    assert.ok(group.inspectedRanges.some(([start, end]) => start <= review.anchor.line && review.anchor.line <= end));
  }
});

test("MP-11 manual patch declaration range uses the physical patch offset", () => {
  const rule = SOURCE_AUDIT_RULES.find((rule) => !rule.embeddedPath && rule.ranges && rule.anchors.length);
  const [category, symbol] = rule.anchors[0];
  const line = rule.ranges[0][0];
  const file = { path: rule.path, blob: rule.blob };
  const relative = ["", "function " + symbol + "() {}"];
  const candidates = sourceRuleCandidates(file, relative, rule.embeddedPath ?? null, line - 2);
  assert.ok(candidates.some((candidate) => candidate.symbol === symbol && candidate.category === category && candidate.lineIndex === 1));
});

test("MP-11 renamed production patch retains old controls when the new suffix is excluded", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("deploy/renamed.patch", ["--- a/apps/api/src/control.ts", "+++ b/apps/api/src/control.md", "@@ -1 +1 @@",
      '-const control = "CHARIOX_MANAGED_OLD_VISIBLE";', '+new documentation'].join("\n"));
    const entry = collect(fixture).entries.find((entry) => entry.selector === "CHARIOX_MANAGED_OLD_VISIBLE");
    assert.ok(entry);
    assert.equal(entry.patchSource.path, "apps/api/src/control.ts");
    assert.equal(entry.sourceRoleHints.executionRole, "removed_patch_source_candidate");
    assert.equal(entry.semanticDisposition.status, "unreviewed");
  });
});

test("MP-11 HEAD scan reads committed blobs when Git suppresses worktree dirt", () => {
  const root = mkdtempSync(join(tmpdir(), "chariox-mp11-immutable-"));
  const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" });
  try {
    git("init", "--quiet");
    mkdirSync(join(root, "apps/kernel/src"), { recursive: true });
    const path = "apps/kernel/src/control.rs";
    writeFileSync(join(root, path), 'const CONTROL: &str = "CHARIOX_MANAGED_COMMITTED";\n');
    git("add", ".");
    git("-c", "user.name=MP-11 fixture", "-c", "user.email=fixture@example.invalid", "commit", "--quiet", "-m", "MP-11 immutable fixture");
    const commit = git("rev-parse", "HEAD").trim();
    const blob = git("rev-parse", "HEAD:" + path).trim();
    git("update-index", "--assume-unchanged", path);
    writeFileSync(join(root, path), 'const CONTROL: &str = "CHARIOX_MANAGED_UNCOMMITTED";\n');
    assert.equal(git("status", "--porcelain"), "");
    const report = collectSourceInventory({ sourceRoot: root, expectedCommit: commit });
    assert.ok(report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_COMMITTED" && entry.blob === blob));
    assert.ok(!report.entries.some((entry) => entry.selector === "CHARIOX_MANAGED_UNCOMMITTED"));
    assert.equal(report.status, "fail");
  } finally { rmSync(root, { recursive: true, force: true }); }
});


test("MP-11 disappeared same-source approval remains an explicit gate blocker", () => {
  withFixture({}, (fixture) => {
    const review = CURRENT_SEMANTIC_REVIEWS[0];
    const runGit = (args, root) => args[0] === "rev-parse"
      ? (args[1] === "HEAD" ? review.sourceCommit : review.sourceTree) + "\n"
      : fixture.runGit(args, root);
    const report = collect(fixture, { runGit, expectedCommit: review.sourceCommit, expectedTree: review.sourceTree });
    assert.ok(report.semanticReviews.some((record) => record.id === review.id && record.status === "source_drift"));
    assert.ok(report.summary.unappliedCurrentSemanticReviews > 0);
    assert.equal(report.status, "fail");
    assert.ok(report.entries.every((entry) => entry.semanticDisposition.status === "unreviewed"));
  });
});

for (const [name, lines] of [
  ["orphan newline marker", ["--- a/apps/api/src/control.ts", "+++ b/apps/api/src/control.ts", "\\ No newline at end of file", "@@ -1 +1 @@", "-old", "+new"]],
  ["empty hunk", ["--- a/apps/api/src/control.ts", "+++ b/apps/api/src/control.ts", "@@ -0,0 +0,0 @@"]],
  ["overlapping hunks", ["--- a/apps/api/src/control.ts", "+++ b/apps/api/src/control.ts", "@@ -1 +1 @@", "-old", "+new", "@@ -1 +1 @@", "-other", "+other"]],
]) test("MP-11 malformed standard patch rejects " + name, () => {
  withFixture({}, (fixture) => {
    fixture.addFile("deploy/invalid.patch", lines.join("\n") + "\n");
    assert.throws(() => collect(fixture), /patch/);
  });
});


for (const [path, blob] of [
  ["apps/kernel/src/runtime/state/testdata/claude-setup-token-2.1.281-start.pty", "d95a0f81eeed8d1ed7d7c015148db4ae94fcecba"],
  ["packages/app-package/fuzz/seeds/archive/0", "26bc3530acf221ec9be6f0aec5dac3afa24ea72c"],
  ["packages/app-package/fuzz/seeds/signed/0", "cc0d165b5ec9e865402a8192fce4003e3a0095a4"],
  ["packages/app-package/fuzz/seeds/signed/1", "d8313ed338ac58bed4f958eed6ed672edc6daabd"],
  ["packages/app-package/fuzz/seeds/signed/2", "c7e62e2614e5743c93982528642ae423805099c9"],
  ["packages/app-package/fuzz/seeds/signed/3", "1fad10dc55a0499ba151d40adfc82277fa5f771d"],
]) test("MP-11 exact fixture exclusion rejects changed blob or executable mode: " + path, () => {
  withFixture({}, (fixture) => {
    fixture.addFile(path, Buffer.from([0]), "100644", blob);
    assert.ok(!collect(fixture).entries.some((entry) => entry.path === path));
    fixture.addFile(path, 'control="CHARIOX_MANAGED_NEW_HIDDEN_CONTROL"\n');
    assert.throws(() => collect(fixture), /excluded fixture source drift/);
    fixture.addFile(path, Buffer.from([0]), "100755", blob);
    assert.throws(() => collect(fixture), /excluded fixture source drift/);
  });
});


for (const path of ["src/new.selector", "packages/app-package/fuzz/seeds/signed/0"]) {
  test("MP-11 patch cannot silently ignore unsupported embedded source: " + path, () => {
    withFixture({}, (fixture) => {
      fixture.addFile("deploy/unsupported.patch", ["--- a/" + path, "+++ b/" + path, "@@ -1 +1 @@", "-old", "+CHARIOX_MANAGED_HIDDEN"].join("\n"));
      assert.throws(() => collect(fixture), /unsupported patch fixture|unclassified production/);
    });
  });
}


test("MP-11 native source symlink cannot be mistaken for an inventoried implementation", () => {
  withFixture({}, (fixture) => {
    fixture.addFile("apps/app-worker/src/control.cc", "outside/control.cc", "120000");
    assert.throws(() => collect(fixture), /unsupported source mode/);
  });
});
