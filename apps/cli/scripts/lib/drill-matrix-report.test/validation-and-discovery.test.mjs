import {
  assert,
  drillMatrixReportCompletionExitCode,
  drillMatrixReportExitCode,
  findDrillMatrixReportPaths,
  formatDrillMatrixAggregateSummary,
  formatDrillMatrixReportSummary,
  mkdtemp,
  os,
  path,
  readDrillMatrixReport,
  rm,
  summarizeDrillMatrixReport,
  summarizeDrillMatrixReports,
  test,
  validateDrillMatrixAggregate,
  validateDrillMatrixReport,
  writeFile,
  matrixReport,
  scenario,
  writeFileWithDir,
} from '../drill-matrix-report.test-support.mjs'

test("rejects inconsistent matrix aggregates", () => {
  const aggregate = summarizeDrillMatrixReports([
    matrixReport({
      matrix: "remote",
      scenarios: [
        scenario("remote", "failed", {
          classification: "provider-auth",
          reason: "expired token",
          runtimeSignals: ["provider-run-lifecycle"],
        }),
      ],
    }),
  ])

  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    totals: { ...aggregate.totals, failed: 2 },
  }), /scenario total does not match status counts/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    totals: { ...aggregate.totals, durationMs: 10.5 },
  }), /aggregate totals has invalid durationMs/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    totals: { ...aggregate.totals, scenarios: 2, failed: 2 },
  }), /totals.scenarios does not match reports/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      scenarioCount: 1.5,
    }],
  }), /reports\[0\] has invalid scenarioCount/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      durationMs: 10.5,
    }],
  }), /reports\[0\] has invalid durationMs/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      counts: { ...aggregate.reports[0].counts, failed: 2 },
    }],
  }), /reports\[0\] scenarioCount does not match counts/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      status: "passed",
    }],
  }), /reports\[0\] status does not match counts/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      counts: { ...aggregate.reports[0].counts, failed: -1 },
    }],
  }), /reports\[0\]\.counts has invalid failed/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      counts: { ...aggregate.reports[0].counts, failed: 0.5 },
    }],
  }), /reports\[0\]\.counts has invalid failed/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    owners: { "runtime-network": 1 },
  }), /owners do not match failedScenarios/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    classifications: { "provider-error": 1 },
  }), /classifications do not match reports/)
  assert.throws(() => validateDrillMatrixAggregate({
    ...aggregate,
    classifications: { "provider-ath": 1 },
  }), /aggregate\.classifications has unknown classification "provider-ath"/)
  assert.throws(() => validateDrillMatrixAggregate({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      classifications: { "provider-ath": 1 },
    }],
  }), /reports\[0\]\.classifications has unknown classification "provider-ath"/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    matrixNames: { remote: 2 },
  }), /matrixNames do not match reports/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    deploymentPresets: { local: 1 },
  }), /deploymentPresets do not match reports/)
  assert.throws(() => validateDrillMatrixAggregate({
    ...aggregate,
    deploymentPresets: { "same-host-remtoe": 1 },
  }), /aggregate\.deploymentPresets\[0\] has unknown deployment preset "same-host-remtoe"/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    providers: { codex: 2 },
  }), /providers do not match reports/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    scenarioIds: { remote: 2 },
  }), /scenarioIds do not match reports/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    exitCriteria: { "dry-run": 1 },
  }), /exitCriteria do not match incompleteExitCriteria/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      deploymentPresets: ["same-host-remtoe"],
    }],
  }), /reports\[0\]\.deploymentPresets\[0\] has unknown deployment preset "same-host-remtoe"/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      exitCriteria: { "not-real": 1 },
    }],
  }), /reports\[0\]\.exitCriteria has invalid status/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    runtimeSignalScenarios: {
      "provider-run-lifecycle": [{
        matrix: "other",
        source: null,
        id: "remote",
        status: "failed",
      }],
    },
  }), /runtimeSignalScenarios do not match reports/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    runtimeSignalScenarios: {
      "provider-run-lifecycle": [{
        matrix: "remote",
        source: null,
        id: "remote",
        status: "passed",
      }],
    },
    reports: [{
      ...aggregate.reports[0],
      runtimeSignalScenarios: {
        "provider-run-lifecycle": [{
          id: "remote",
          status: "passed",
        }],
      },
    }],
  }), /runtimeSignalScenarios status does not match scenario diagnostics for remote\/remote/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    runtimeSignalOwners: { "kernel-authority": 1 },
  }), /runtimeSignalOwners do not match runtimeSignals/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    runtimeSignals: { "workspace-live-synch-state": 1 },
  }), /aggregate\.runtimeSignals has unknown runtime signal "workspace-live-synch-state"/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      runtimeSignals: { "workspace-live-synch-state": 1 },
    }],
  }), /reports\[0\]\.runtimeSignals has unknown runtime signal "workspace-live-synch-state"/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      runtimeSignalScenarios: {
        "provider-run-lifecycle": [],
      },
    }],
  }), /runtimeSignalScenarios\.provider-run-lifecycle has invalid scenarios/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      runtimeSignalScenarios: {
        "provider-run-lifecycle": [{
          id: "other",
          status: "failed",
        }],
      },
    }],
  }), /reports\[0\]\.runtimeSignalScenarios references unknown scenario "other"/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      scenarioIds: [],
    }],
  }), /reports\[0\] scenarioIds do not match scenarioCount/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    nextActions: [],
  }), /nextActions do not match failedScenarios/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    failedScenarios: [{
      ...aggregate.failedScenarios[0],
      classification: "not-real",
    }],
  }), /failedScenarios\[0\] has unknown classification/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    failedScenarios: [{
      ...aggregate.failedScenarios[0],
      owner: "runtime-network",
    }],
  }), /failedScenarios\[0\] owner does not match classification/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    failedScenarios: [{
      ...aggregate.failedScenarios[0],
      nextAction: "try something else",
    }],
  }), /failedScenarios\[0\] nextAction does not match classification/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    failedScenarios: [{
      ...aggregate.failedScenarios[0],
      artifactHints: ["Bearer abcdefghijklmnopqrstuvwxyz"],
    }],
  }), /failedScenarios\[0\] includes secret-looking artifactHints/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    status: "passed",
  }), /status does not match totals/)
})

test("rejects unknown deployment preset labels in report metadata", () => {
  assert.throws(
    () => validateDrillMatrixReport(matrixReport({
      metadata: {
        deploymentPresets: "local,same-host-remtoe",
        deploymentPresetCount: 2,
      },
    })),
    /metadata\.deploymentPresets\[1\] has unknown deployment preset "same-host-remtoe"/,
  )
  assert.throws(
    () => validateDrillMatrixReport(matrixReport({
      metadata: {
        deploymentPresets: "local,hetzner",
        deploymentPresetCount: 1,
      },
    })),
    /metadata\.deploymentPresetCount does not match deploymentPresets/,
  )
})

test("reads and validates report files", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-report-"))
  const file = path.join(dir, "matrix.json")
  await writeFile(file, `${JSON.stringify(matrixReport())}\n`, "utf8")

  const report = await readDrillMatrixReport(file)

  assert.equal(report.schema, "arroba.drill.matrix.v1")
  assert.throws(() => validateDrillMatrixReport({ schema: "other", scenarios: [] }), /unsupported schema/)
  await rm(dir, { recursive: true, force: true })
})

test("discovers matrix reports below artifact roots", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-report-find-"))
  const first = path.join(dir, ".artifacts", "drill-matrices", "one", "matrix.json")
  const second = path.join(dir, ".artifacts", "drill-matrices", "two", "matrix.json")
  const unrelated = path.join(dir, ".artifacts", "drill-matrices", "two", "other.json")
  const pruned = path.join(dir, "node_modules", "package", "matrix.json")
  const deep = path.join(dir, "one", "two", "three", "four", "matrix.json")
  await writeFileWithDir(first, `${JSON.stringify(matrixReport({ matrix: "one" }))}\n`)
  await writeFileWithDir(second, `${JSON.stringify(matrixReport({ matrix: "two" }))}\n`)
  await writeFileWithDir(unrelated, `${JSON.stringify({ schema: "other" })}\n`)
  await writeFileWithDir(pruned, `${JSON.stringify(matrixReport({ matrix: "pruned" }))}\n`)
  await writeFileWithDir(deep, `${JSON.stringify(matrixReport({ matrix: "deep" }))}\n`)

  const reports = await findDrillMatrixReportPaths([path.join(dir, ".artifacts")])
  const broadReports = await findDrillMatrixReportPaths([dir])
  const shallowReports = await findDrillMatrixReportPaths([dir], { maxDepth: 2 })

  assert.deepEqual(reports, [first, second].sort())
  assert.deepEqual(broadReports, [deep, first, second].sort())
  assert.deepEqual(shallowReports, [])
  await rm(dir, { recursive: true, force: true })
})

test("rejects malformed matrix reports", () => {
  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    status: "unknown",
  }), /invalid status/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({ scenarios: [] }),
  }), /has no scenarios/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      status: "passed",
      scenarios: [scenario("remote", "failed", { classification: "provider-auth", reason: "expired token" })],
    }),
  }), /status does not match scenario statuses/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      status: "dry-run",
      dryRun: false,
      scenarios: [scenario("remote", "dry-run")],
    }),
  }), /dryRun does not match scenario statuses/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("remote", "passed", {
        exitCriteria: ["provider turn completes"],
        exitCriteriaEvidence: [{
          id: "remote:exit-01",
          criterion: "provider turn completes",
          status: "satisfied",
          reason: null,
        }],
      })],
      exitCriteria: { failed: 1 },
    }),
  }), /exitCriteria do not match scenario exit criteria evidence/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      status: "dry-run",
      dryRun: true,
      scenarios: [scenario("remote", "dry-run", {
        exitCriteria: ["provider turn completes"],
        exitCriteriaEvidence: [{
          id: "remote:exit-01",
          criterion: "provider turn completes",
          status: "dry-run",
          reason: "scenario command was selected but not executed",
        }],
      })],
      incompleteExitCriteria: [{
        scenarioId: "other",
        id: "remote:exit-01",
        criterion: "provider turn completes",
        status: "dry-run",
        reason: "scenario command was selected but not executed",
      }],
    }),
  }), /incompleteExitCriteria do not match scenario exit criteria evidence/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("remote", "passed", { runtimeSignals: ["session-authority"] })],
      runtimeSignals: { "lease-health": 1 },
    }),
  }), /runtimeSignals do not match scenario runtimeSignals/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("remote", "passed", { runtimeSignals: ["session-authority"] })],
      runtimeSignalOwners: { "kernel-authority": 1 },
    }),
  }), /runtimeSignalOwners requires runtimeSignals/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("remote", "passed", { runtimeSignals: ["session-authority"] })],
      runtimeSignals: { "session-authority": 1 },
      runtimeSignalOwners: { "runtime-network": 1 },
    }),
  }), /runtimeSignalOwners do not match runtimeSignals/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("remote", "passed", { runtimeSignals: ["session-authority"] })],
      runtimeSignals: { "session-authority": 1 },
      runtimeSignalScenarios: {
        "session-authority": [{ id: "remote", status: "failed" }],
      },
    }),
  }), /runtimeSignalScenarios do not match scenario runtimeSignals/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("remote", "passed", { provider: "cdoex", providers: ["cdoex"] })],
    }),
  }), /scenarios\[0\]\.provider\[0\] has unknown provider "cdoex"/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("remote", "passed", {
        provider: "codex",
        providers: ["opencode"],
      })],
      metadata: { providers: "opencode" },
    }),
  }), /scenarios\[0\]\.provider must be included in providers/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("remote", "passed", { deployment: "Bearer abcdefghijklmnop" })],
    }),
  }), /scenarios\[0\] has invalid deployment/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("remote", "passed", { mode: "" })],
    }),
  }), /scenarios\[0\] has invalid mode/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    durationMs: -1,
  }), /invalid durationMs/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      durationMs: 1000.5,
      completedAt: "2026-06-13T00:00:01.500Z",
    }),
  }), /invalid durationMs/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    durationMs: 999,
  }), /durationMs must match completedAt - startedAt/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    startedAt: "2026-06-13",
  }), /startedAt must be an ISO timestamp/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    startedAt: "2026-06-13T00:00:02.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
  }), /completedAt must not be before startedAt/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [{ ...scenario("broken", "passed"), command: "" }],
  }), /scenarios\[0\] is missing command/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [{ ...scenario("broken", "passed"), args: [1] }],
  }), /scenarios\[0\] has invalid args/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [{ ...scenario("broken", "passed"), exitCriteria: [1] }],
  }), /scenarios\[0\] has invalid exitCriteria/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [scenario("broken", "passed", {
      exitCriteria: ["criterion"],
      exitCriteriaEvidence: [],
    })],
  }), /scenarios\[0\]\.exitCriteriaEvidence length does not match exitCriteria/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [scenario("broken", "passed", {
      exitCriteria: ["criterion"],
      exitCriteriaEvidence: [{
        id: "broken:exit-01",
        criterion: "different criterion",
        status: "satisfied",
        reason: null,
      }],
    })],
  }), /scenarios\[0\]\.exitCriteriaEvidence\[0\] criterion does not match exitCriteria/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [scenario("broken", "passed", {
      exitCriteria: ["criterion"],
      exitCriteriaEvidence: [{
        id: "broken:exit-01",
        criterion: "criterion",
        status: "dry-run",
        reason: null,
      }],
    })],
  }), /scenarios\[0\]\.exitCriteriaEvidence\[0\] incomplete criterion is missing reason/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [{ ...scenario("broken", "passed"), artifactHints: [1] }],
  }), /scenarios\[0\] has invalid artifactHints/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [{ ...scenario("broken", "passed"), artifactHints: ["/tmp/arroba-drill-sk-this-should-not-persist"] }],
  }), /scenarios\[0\] includes secret-looking artifactHints/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [{ ...scenario("broken", "passed"), artifactHints: [{ kind: "manifest", path: "Bearer abcdefghijklmnopqrstuvwxyz" }] }],
  }), /scenarios\[0\] includes secret-looking artifactHints/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "failed", { classification: "child-process", reason: "" })],
    }),
  }), /scenarios\[0\] failed scenario is missing reason/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "failed", { reason: "code=1" })],
    }),
  }), /scenarios\[0\] failed scenario is missing classification/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "failed", { classification: "typo-runtime", reason: "code=1" })],
    }),
  }), /scenarios\[0\] has unknown classification "typo-runtime"/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "failed", {
        classification: "provider-auth",
        owner: "runtime-network",
        reason: "expired token",
      })],
    }),
  }), /scenarios\[0\] owner does not match classification/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "failed", {
        classification: "provider-auth",
        nextAction: "try something else",
        reason: "expired token",
      })],
    }),
  }), /scenarios\[0\] nextAction does not match classification/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "skipped", { reason: null })],
    }),
  }), /scenarios\[0\] skipped scenario is missing reason/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "skipped", { reason: "not selected", durationMs: 1 })],
    }),
  }), /scenarios\[0\] skipped scenario must have zero durationMs/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "passed", { durationMs: 1.5 })],
    }),
  }), /scenarios\[0\] has invalid durationMs/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      status: "dry-run",
      dryRun: true,
      scenarios: [scenario("broken", "dry-run", { durationMs: 1 })],
    }),
  }), /scenarios\[0\] dry-run scenario must have zero durationMs/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      status: "dry-run",
      dryRun: true,
      scenarios: [scenario("broken", "dry-run", { reason: "not run" })],
    }),
  }), /scenarios\[0\] dry-run scenario must not include reason/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      status: "dry-run",
      dryRun: true,
      scenarios: [scenario("broken", "dry-run", { classification: "expected-failure" })],
    }),
  }), /scenarios\[0\] dry-run scenario must not include classification/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "passed", { reason: "unexpected warning" })],
    }),
  }), /scenarios\[0\] passed scenario must not include reason/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { relayToken: "redacted-or-not-it-should-not-be-here" },
  }), /sensitive metadata key/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { provider: "Bearer abcdefghijklmnopqrstuvwxyz" },
  }), /secret-looking metadata value/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { providers: "codex,opencode", providerCount: 3 },
  }), /metadata\.providerCount does not match providers/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { providers: "codex,cdoex", providerCount: 2 },
  }), /metadata\.providers\[\d+\] has unknown provider "cdoex"/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { providers: "codex", providerModelOverrides: "opencode" },
  }), /metadata\.providerModelOverrides includes provider not in providers/)

  assert.doesNotThrow(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { providers: "codex,opencode", providerAccountAliases: "codex=work" },
  }))

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { providers: "codex", providerAccountAliases: "opencode=zen" },
  }), /metadata\.providerAccountAliases includes provider not in providers/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { providers: "codex", providerAccountAliases: "codex=user@example.test" },
  }), /metadata\.providerAccountAliases includes invalid account alias/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      metadata: { providers: "codex,opencode", providerCount: 2 },
      scenarios: [scenario("local", "passed", { providers: ["codex", ""] })],
    }),
  }), /scenarios\[0\]\.providers has invalid providers/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      metadata: { providers: "codex", providerCount: 1 },
      scenarios: [scenario("local", "passed", { providers: ["codex", "cdoex"] })],
    }),
  }), /scenarios\[0\]\.providers\[1\] has unknown provider "cdoex"/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      metadata: { providers: "codex,opencode", providerCount: 2 },
      scenarios: [scenario("local", "passed", { providers: ["codex"] })],
    }),
  }), /metadata\.providers do not match scenario providers/)

  const aggregate = summarizeDrillMatrixReports([matrixReport({
    metadata: { providers: "codex", providerCount: 1 },
    scenarios: [scenario("local", "passed", { providers: ["codex"] })],
  })])
  assert.throws(() => validateDrillMatrixAggregate({
    ...aggregate,
    providers: { cdoex: 1 },
  }), /aggregate\.providers\[0\] has unknown provider "cdoex"/)
})

