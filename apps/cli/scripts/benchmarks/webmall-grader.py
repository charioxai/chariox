"""MP-08 / MP-10: unchanged WebMallTask evaluator on existing Room pages.

CDP attaches through an owned, unpublished TCP forwarder. No solver actions,
HTTP shopping, new browser, or grading feedback to the model. Only setup()
navigates, before the provider prompt. Expected answers stay on the host.
"""
import contextlib
import csv
import io
import json
import os
from pathlib import Path
import sys
import urllib.request
from playwright.sync_api import sync_playwright
from browsergym.webmall.task import WebMallTask
sys.path.insert(0, str(Path(__file__).resolve().parent / "round2"))
from webmall_sampling import fresh_existing_pages

os.environ.update(json.loads(Path(sys.argv[1]).read_text()))
sys.path.insert(0, sys.argv[2])
from analyze_agentlab_results.summarize_study import calculation_results
endpoint, output = sys.argv[3], Path(sys.argv[4])
output.mkdir(parents=True, exist_ok=True, mode=0o700)
info = json.load(urllib.request.urlopen(endpoint + "/json/version", timeout=10))
websocket = endpoint.replace("http:", "ws:") + "/devtools/browser/" + info["webSocketDebuggerUrl"].split("/devtools/browser/")[1]
with sync_playwright() as pw:
    browser = pw.chromium.connect_over_cdp(websocket)
    context = browser.contexts[0]
    context.set_default_timeout(2000)
    task, samples, finished, final_wrong = None, 0, False, []
    for line in sys.stdin:
        request = json.loads(line)
        try:
            op = request["op"]
            if op == "setup":
                pages = [p for p in context.pages if not p.url.startswith("chrome-extension:")]
                assert len(pages) == 1, "MP-10 setup requires one existing Room page"
                task = WebMallTask("webmall." + request["task"], seed=42)
                goal, _ = task.setup(pages[0])
                assert goal == request["goal"], "MP-10 public instruction mismatch"
                result = {"setup": True}
            elif op in ["sample", "finish"]:
                assert task is not None
                wrong, sample_infos, observation_errors = list(final_wrong), [], []
                # Read every existing page; never navigate or create a page.
                for page in ([] if finished else fresh_existing_pages(
                        context, strict=op == "finish", on_error=observation_errors.append)):
                    if page.url.startswith("chrome-extension:"): continue
                    try:
                        with contextlib.redirect_stdout(io.StringIO()):
                            _, done, _, entry = task.validate(page, [])
                        wrong.extend(entry.get("wrong_solutions") or [])
                        sample_infos.append(entry)
                        finished = finished or done
                        if done:
                            # BrowserGym terminates on this exact validation. Repeating
                            # StringEvaluator after done would mislabel already-checked
                            # correct URLs as extras because routing uses unchecked CPs.
                            final_wrong = list(wrong)
                            break
                    except Exception:
                        # Navigation is an expected transient during sampling.
                        if op == "finish": raise
                if sample_infos: samples += 1
                if op == "sample":
                    result = {"sampled": True, "pages": len(sample_infos),
                              "observationErrors": observation_errors}
                else:
                    checklist = task.checklist.get_checklist_dict()
                    answer = [c for c in checklist if c["id"].startswith("answer")]
                    benchmark = {c["id"] for c in answer}
                    reached = {c["id"] for c in answer if c["flag"]}
                    transaction = task.task_config["category"] in ["Add_To_Cart", "Checkout", "FindAndOrder"]
                    # Match the official summarizer's transaction penalty rule.
                    predicted = reached | (set() if transaction else set(wrong))
                    metrics = calculation_results([benchmark], [predicted])
                    result = {"done": finished, "samples": samples,
                              "officialMetrics": metrics, "answerCount": len(answer),
                              "reachedAnswerCount": len(reached), "wrongCountAtFinish": len(set(wrong)),
                              "strictCompletion": reached == benchmark and not wrong,
                              "transactionPenaltiesExcludedByOfficialSummary": transaction,
                              "weightedCheckpointReward": task.checklist.total_score()}
                    # No expected URL or product details are retained in solver evidence.
                    result["checkpointFlags"] = [{"id": c["id"], "flag": c["flag"], "type": c["type"]} for c in checklist]
                    with (output / "goal_achievement.csv").open("w") as f:
                        writer = csv.writer(f); writer.writerow(["goal_description", "achieved"])
                        writer.writerows((c["id"], c["flag"]) for c in answer)
                    with (output / "penalties.csv").open("w") as f:
                        writer = csv.writer(f); writer.writerow(["wrong_solutions"]); writer.writerow(["|".join(wrong)])
                    screenshots = []
                    for i, page in enumerate(context.pages):
                        if page.url.startswith("chrome-extension:"): continue
                        target = output / f"final-{i}.png"
                        page.screenshot(path=str(target)); screenshots.append(target.name)
                    result["screenshots"] = screenshots
                    (output / "official-grade.json").write_text(json.dumps({"mp_items": ["MP-08", "MP-10"], **result}, indent=2) + "\n")
            else:
                raise ValueError("MP-10 unknown operation")
            print(json.dumps({"ok": True, **result}), flush=True)
        except Exception as error:
            print(json.dumps({"ok": False, "operation": request["op"], "errorClass": type(error).__name__}), flush=True)
    if task: task.teardown()
    # Detach Playwright; Chromium remains under product lifecycle ownership.
