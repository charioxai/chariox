"""MP-08 / MP-10: export only the official public WebMall setup instruction."""
import json
import os
from pathlib import Path
import sys
from browsergym.webmall.task import WebMallTask

urls = json.loads(Path(sys.argv[1]).read_text())
os.environ.update(urls)
source = Path(sys.argv[2])
sets = json.loads((source / "browsergym/webmall/src/browsergym/webmall/task_sets.json").read_text())

class SetupPage:
    # Official setup only navigates to the frontend. No browser is launched.
    def goto(self, url, **kwargs):
        assert url == urls["FRONTEND_URL"]

rows = []
for group in sets:
    for metadata in group["tasks"]:
        task = WebMallTask("webmall." + metadata["id"], seed=42)
        goal, _ = task.setup(SetupPage())
        assert "{{" not in goal
        rows.append({"task_id": metadata["id"], "category": metadata["category"],
                     "intent": goal, "start_urls": [urls["FRONTEND_URL"]]})
assert len(rows) == 91
Path(sys.argv[3]).write_text(json.dumps(rows, indent=2) + "\n")
print("MP-08 / MP-10 exported 91 public instructions; no grading fields")
