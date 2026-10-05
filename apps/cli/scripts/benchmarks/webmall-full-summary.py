"""MP-08 / MP-10: official full-set aggregation; never read expected answers."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import statistics
import sys

parser = argparse.ArgumentParser()
parser.add_argument("--evidence", type=Path, required=True)
parser.add_argument("--upstream", type=Path, required=True)
args = parser.parse_args()
sys.path.insert(0, str(args.upstream))
from analyze_agentlab_results.summarize_study import calculation_results

rows = json.loads((args.evidence / "RESULTS.json").read_text())
manifest = json.loads((args.evidence / "FULL_MANIFEST.json").read_text())
assert {r["taskId"] for r in rows} == set(manifest["tasks"])
attempt_rows = rows
by_task = {}
for row in attempt_rows:
    previous = by_task.get(row["taskId"])
    if previous:
        assert not previous.get("promptId"), "Never select a second provider attempt"
        assert previous["status"] == "RED" and previous.get("firstFailingSeam") in ["room_create", "slice_create", "slice_start", "grader_attach"]
        assert row["harnessRetry"]["retryOf"] == previous["runId"]
        assert not row["harnessRetry"]["providerPromptRepeated"]
    by_task[row["taskId"]] = row
rows = list(by_task.values())
assert len(rows) == 91
assert {r["taskId"] for r in rows if r["scope"] == "smoke"} == set(manifest["carriedSmoke"])
assert all(r.get("finishedAt") for r in rows)
benchmark, predictions, evaluated = [], [], []
for row in rows:
    grade = row.get("grade") or {}
    if not grade.get("ok"):
        continue
    answers = [c for c in grade["checkpointFlags"] if c["id"].startswith("answer")]
    expected = {c["id"] for c in answers}
    predicted = {c["id"] for c in answers if c["flag"]}
    if not grade["transactionPenaltiesExcludedByOfficialSummary"]:
        predicted |= {f"wrong:{i}" for i in range(grade["wrongCountAtFinish"])}
    assert calculation_results([expected], [predicted]) == grade["officialMetrics"]
    benchmark.append(expected)
    predictions.append(predicted)
    evaluated.append(row)
metrics = calculation_results(benchmark, predictions) if evaluated else None
valid = [r for r in rows if r["harnessValid"]]
invalid = [{"taskId": r["taskId"], "firstFailingSeam": r.get("firstFailingSeam"),
            "failure": r.get("failure"), "promptAdmitted": bool(r.get("promptId")),
            "officialGradeAvailable": bool((r.get("grade") or {}).get("ok"))}
           for r in rows if not r["harnessValid"]]
summary = {
    "mp_items": ["MP-08", "MP-10"], "at": datetime.now(timezone.utc).isoformat(),
    "scope": "full-set-local-round1", "tasks": 91, "carriedSmokeTasks": 10,
    "initialContinuationAttempts": 81, "attemptReceipts": len(attempt_rows),
    "harnessRetries": [r["harnessRetry"] | {"taskId": r["taskId"]} for r in attempt_rows if r.get("harnessRetry")],
    "validHarnessTasks": len(valid), "officiallyEvaluatedTasks": len(evaluated),
    "completeValidRun": len(valid) == 91 and len(evaluated) == 91,
    "officialMetrics": metrics, "officialMetricsDenominator": len(evaluated),
    "officialCompletions": sum(r["grade"]["officialMetrics"]["avg_task_completion_rate"] == 1 for r in evaluated),
    "meanPerTaskF1": statistics.mean(r["grade"]["officialMetrics"]["avg_f1_score"] for r in evaluated) if evaluated else None,
    "invalidTasks": invalid,
    "missingOfficialGradeTaskIds": [r["taskId"] for r in rows if not (r.get("grade") or {}).get("ok")],
    "taskExclusions": [],
    "transactionPenaltyExclusions": [r["taskId"] for r in evaluated if r["grade"]["transactionPenaltiesExcludedByOfficialSummary"]],
    "leaderboard": json.loads((args.evidence / "LEADERBOARD_SNAPSHOT.json").read_text()),
    "demand": {},
    "resultsSha256": hashlib.sha256((args.evidence / "RESULTS.json").read_bytes()).hexdigest(),
    "officialSummarizerSha256": hashlib.sha256((args.upstream / "analyze_agentlab_results/summarize_study.py").read_bytes()).hexdigest(),
    "limits": ["Local native Chariox tools/observations/budgets are not equivalent to published AgentLab baseline tracks",
               "Existing-browser CDP validation samples between native actions; not exact BrowserGym step boundaries",
               "All available official grades, including RED/provider outcomes, retained; missing grades explicitly listed",
               "Official F1 is harmonic mean of macro precision and recall; mean per-task F1 also reported",
               "Root code/data redistribution grant unspecified; no public submission or upload",
               "This run closes no MP item and benchmarks never block functional merges"]}
summary["leaderboard"]["wouldBeRank"] = None
summary["leaderboard"]["rankReason"] = "Full-set local native Browser track differs from published observation/action/budget tracks; no official submission."
for field in ["providerWallSeconds", "totalWallSeconds", "usageTokensTotal", "providerToolCalls", "actions"]:
    values = [r[field] for r in rows if isinstance(r.get(field), (int, float))]
    summary["demand"][field] = {"observed": len(values), "sum": sum(values), "mean": statistics.mean(values) if values else None}
(args.evidence / "FULL_SUMMARY.json").write_text(json.dumps(summary, indent=2) + "\n")
print("MP-08 / MP-10 full set: " + str(len(valid)) + "/91 harness-valid; " + str(len(evaluated)) + "/91 grades; official metrics " + json.dumps(metrics))
