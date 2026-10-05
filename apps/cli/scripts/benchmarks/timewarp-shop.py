"""MP-08 / MP-10: unchanged official Shop, lane port and pinned site seed.

The process restarts before each episode. No task reference data is loaded by
this adapter, and no reward or normalizer implementation is changed.
"""
import os
import random
import sys

# Pyserini's eager, unused encoder constructor must not require real credentials.
# Shop search remains official local Lucene; any encoder request fails locally.
os.environ["OPENAI_API_KEY"] = "benchtw-unused-local-lucene"
os.environ["OPENAI_BASE_URL"] = "http://127.0.0.1:9"
root, era, port = sys.argv[1:]
os.chdir(root)
sys.path.insert(0, root)
sys.argv = ["app.py", era, "--port=" + port]
random.seed(42)
from web_agent_site.app import app
app.run(host="0.0.0.0", port=int(port), debug=False, use_reloader=False)
