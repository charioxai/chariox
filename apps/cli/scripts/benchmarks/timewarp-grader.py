"""MP-08 / MP-10: unchanged TimeWarp setup/validate on the existing Room page.

The external evaluator returns public goals and verifier types only. Gold
answers remain private; scoring runs once after the provider has settled.
No browser is launched and no solver action is performed here.
"""
import importlib.resources
import json
import os
import sys

os.environ["NLTK_DATA"] = "/tmp/benchtw/nltk"
import browsergym.timewarp
from browsergym.timewarp.task import GenericTimeWarpTask
from playwright.sync_api import sync_playwright


def main():
    configs = json.loads(importlib.resources.files(browsergym.timewarp)
                         .joinpath("data/test.raw.json").read_text())
    with sync_playwright() as pw:
        browser = pw.chromium.connect_over_cdp("http://127.0.0.1:9222")
        context = browser.contexts[0]
        pages = [p for p in context.pages if not p.url.startswith("chrome-extension:")]
        if len(pages) != 1:
            raise RuntimeError("MP-10 requires one existing Room page")
        page, task, graded = pages[0], None, False
        for line in sys.stdin:
            request = json.loads(line)
            try:
                match request["op"]:
                    case "manifest":
                        result = {"count": len(configs), "episodesAcrossSixEras": len(configs) * 6,
                                  "tasks": [{"taskId": c["task_id"], "sites": c["sites"],
                                             "evalTypes": c["eval"]["eval_types"]} for c in configs]}
                    case "setup":
                        if task:
                            task.teardown()
                        for site, url in request["urls"].items():
                            os.environ["TW_" + site.upper()] = url
                        task = GenericTimeWarpTask(seed=request["seed"], task_id=request["taskId"])
                        config = task.task_configs[0]
                        if "llm_judge" in config["eval"]["eval_types"]:
                            raise RuntimeError("MP-10 smoke does not admit an unconfigured judge")
                        # Ordinary fixture reset, before official task setup.
                        context.clear_cookies()
                        for other in list(context.pages):
                            if other != page:
                                other.close()
                        page.goto("about:blank")
                        page.set_viewport_size(task.viewport)
                        page.set_default_timeout(task.timeout)
                        goal, info = task.setup(page)
                        result = {"goal": goal, "info": info, "url": page.url,
                                  "evalTypes": config["eval"]["eval_types"], "viewport": task.viewport}
                        graded = False
                    case "validate":
                        if graded:
                            raise RuntimeError("MP-10 only one grading attempt per episode")
                        graded = True
                        reward, done, _, info = task.validate(page, [
                            {"role": "assistant", "message": request["answer"]}])
                        result = {"reward": reward, "done": bool(done), "info": info, "url": page.url}
                    case "screenshot":
                        page.screenshot(path=request["path"])
                        result = {"captured": True}
                    case _:
                        raise ValueError("unknown evaluator operation")
                print(json.dumps({"ok": True, **result}), flush=True)
            except Exception as error:
                print(json.dumps({"ok": False, "error_class": type(error).__name__,
                                  "operation": request["op"]}), flush=True)
        if task:
            task.teardown()


if __name__ == "__main__":
    main()
