"""MP-08 / MP-10: official MiniWoB setup/grading on the existing Room page.

This process is an external evaluator. It never performs solver actions, exposes
grader state to the model, launches Chromium, or alters the upstream reward.
"""
import ast
import csv
import json
from pathlib import Path

import numpy as np
import sys

from browsergym.miniwob import ALL_MINIWOB_TASKS
from playwright.sync_api import sync_playwright


def main():
    tasks = {task.get_task_id(): task for task in ALL_MINIWOB_TASKS}
    with sync_playwright() as pw:
        browser = pw.chromium.connect_over_cdp("http://127.0.0.1:9222")
        context = browser.contexts[0]
        pages = [page for page in context.pages if not page.url.startswith("chrome-extension:")]
        if len(pages) != 1:
            raise RuntimeError("grader requires exactly one existing Room page")
        page = pages[0]
        task = None
        for line in sys.stdin:
            request = json.loads(line)
            try:
                match request["op"]:
                    case "manifest":
                        result = {"templates": sorted(tasks), "count": len(tasks)}
                    case "schedule":
                        # Execute the pinned official seed expression and repeat function.
                        # EnvArgs is a serialization stub; no solver or reward changes.
                        base = Path("/tmp/benchmini/browsergym/experiments/src/browsergym/experiments")
                        loop = ast.parse((base / "loop.py").read_text())
                        seed_node = next(node for node in loop.body if isinstance(node, ast.Assign)
                                         and any(isinstance(t, ast.Name) and t.id == "SEED_MAX" for t in node.targets))
                        funcs = ast.parse((base / "benchmark/utils.py").read_text())
                        repeat_node = next(node for node in funcs.body if isinstance(node, ast.FunctionDef)
                                           and node.name == "make_env_args_list_from_repeat_tasks")
                        namespace = {"np": np, "EnvArgs": lambda **kwargs: kwargs}
                        exec(compile(ast.Module(body=[seed_node, repeat_node], type_ignores=[]),
                                     "official-schedule", "exec"), namespace)
                        with (base / "benchmark/metadata/miniwob.csv").open() as metadata:
                            names = [row["task_name"] for row in csv.DictReader(metadata)]
                        if set(names) != set(tasks) or len(names) != 125:
                            raise RuntimeError("official metadata and task registry differ")
                        episodes = namespace["make_env_args_list_from_repeat_tasks"](
                            task_list=names, max_steps=10, n_repeats=5, seeds_rng=np.random.RandomState(42))
                        for index, episode in enumerate(episodes):
                            episode.update(index=index, repeat=index % 5)
                        result = {"episodes": episodes, "seed_max": namespace["SEED_MAX"]}
                    case "setup":
                        if task is not None:
                            task.teardown()
                        task = tasks[request["task"]](
                            seed=request["seed"], base_url="http://127.0.0.1:8765/miniwob/",
                            episode_max_time=request["episode_max_time_ms"],
                        )
                        page.set_viewport_size(task.viewport)
                        page.set_default_timeout(task.timeout)
                        goal, info = task.setup(page)
                        result = {"goal": goal, "info": info, "url": page.url,
                                  "viewport": task.viewport}
                    case "validate":
                        reward, done, _, info = task.validate(page, [])
                        result = {"reward": reward, "done": bool(done), "info": info,
                                  "url": page.url}
                    case "screenshot":
                        page.screenshot(path=request["path"])
                        result = {"captured": True}
                    case _:
                        raise ValueError("unknown evaluator operation")
                print(json.dumps({"ok": True, **result}), flush=True)
            except Exception as error:
                # Fixed diagnostics: no arbitrary page/provider/config content.
                print(json.dumps({"ok": False, "error_class": type(error).__name__,
                                  "operation": request["op"]}), flush=True)
        if task is not None:
            task.teardown()
        # Stopping Playwright detaches; the product retains browser lifecycle.


if __name__ == "__main__":
    main()
