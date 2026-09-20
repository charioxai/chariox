import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import {
  collectSourceInventory,
  MP_ROWS,
  reviewedPredicatesFor,
  stableJson,
} from "./managed-parity-source-inventory.mjs";

const COMMIT = "fixture-commit-000000000000000000000000000000000000";
const TREE = "fixture-tree-0000000000000000000000000000000000000";

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
    "apps/kernel/src/provider/managed_isolation.rs": [
      "fn managed_mode_fixture() {",
      hiddenManagedFlag ? "  // CHARIOX_MANAGED_HIDDEN_FLAG" : "  // ordinary reviewed selector fixture",
      directBwrap ? "  let command = \"/usr/bin/bwrap\";" : "  let command = \"ordinary-provider\";",
      "  if (is_managed) { run_managed(); }",
      "}",
    ].join("\n") + "\n",
    "deploy/managed-kernel/provider.service": [
      inheritedRestriction ? "RestrictAddressFamilies=AF_UNIX" : "NoNewPrivileges=true",
      "ProtectSystem=strict",
    ].join("\n") + "\n",
    "apps/kernel/src/managed_context/protected.rs": protectedParent ? "fn protected_parent_filter() { /* protected path */ }\n" : "fn ordinary_path_filter() {}\n",
    "apps/kernel/src/error_map.rs": errorMapping ? "// managed failure token=secret-value\n" : "// ordinary error\n",
    "apps/kernel/src/path.rs": "const ROOT = CHARIOX_MANAGED_REPOSITORY_ROOT; // /home/chariox and /tmp\n",
    "apps/kernel/src/cleanup.rs": "// managed cleanup revoke expires the private receipt\n",
    "apps/cli/src/client.ts": "// managed client projection is reviewed separately\n",
    "apps/kernel/slice-linux-docker/docker/Dockerfile": "RUN bwrap --unshare-user --die-with-parent\n",
  };
  if (unknownProjection) files["apps/client/unknown.ts"] = "const value = { managed: projection };\n";
  return files;
}

function makeFixture(options = {}) {
  const root = mkdtempSync(join(tmpdir(), "chariox-mp11-"));
  const files = fixtureFiles(options);
  const entries = [];
  let index = 1;
  for (const [file, contents] of Object.entries(files)) {
    const absolute = join(root, file);
    mkdirSync(dirname(absolute), { recursive: true });
    writeFileSync(absolute, contents);
    entries.push(`100644 blob ${String(index).padStart(40, "0")}\t${file}`);
    index += 1;
  }
  const runGit = (args) => {
    if (args[0] === "rev-parse" && args[1] === "HEAD") return `${COMMIT}\n`;
    if (args[0] === "rev-parse" && args[1] === "HEAD^{tree}") return `${TREE}\n`;
    if (args[0] === "status") return "";
    if (args[0] === "ls-tree") return `${entries.join("\0")}\0`;
    throw new Error(`unexpected git command in fixture: ${args.join(" ")}`);
  };
  const predicates = reviewedPredicatesFor({ sourceCommit: COMMIT, sourceTree: TREE }).map((predicate) => ({
    ...predicate,
    path: predicate.id.startsWith("MP-07")
      ? "apps/kernel/src/managed_bootstrap/release.rs"
      : "apps/kernel/src/runtime/managed_environment_control/cloud_contract.rs",
    line: predicate.id.startsWith("MP-07") ? 39 : predicate.id.endsWith("idle-delay") ? 111 : 110,
    symbol: predicate.id.startsWith("MP-07") ? "verify_release" : "AutoStopPolicy",
  }));
  return { root, files, runGit, predicates };
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
    reviewedPredicates: fixture.predicates,
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

test("current reviewed fixture emits source identity, all MP rows, and exact exemptions", () => {
  withFixture({}, (fixture) => {
    const report = collect(fixture);
    assert.equal(report.source.commit, COMMIT);
    assert.equal(report.source.tree, TREE);
    assert.deepEqual(report.missingCategories, []);
    assert.deepEqual(report.missingRows, []);
    assert.equal(report.summary.allowedReleaseDeployment, 1);
    assert.equal(report.summary.requiredAutomaticShutdown, 2);
    assert.equal(report.status, "fail", "unapproved managed selectors must fail closed");
    assert.ok(report.entries.some((entry) => entry.category === "managed_env_selector"));
    assert.ok(report.entries.some((entry) => entry.category === "cleanup_selector"));
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
    writeFileSync(releasePath, original.replace("pub(super) fn verify_release(", "pub(super) fn verify_release( // drift"));
    const report = collect(fixture);
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
