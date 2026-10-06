"""MP-08 / MP-10: run the unchanged official site on a lane-selected port.

Only the bind address changes so the Room slice can reach its benchmark site.
Source, templates, indexes and browser actions remain upstream-owned.
"""
import importlib.util
import pathlib
import sys

root, site, era, port = sys.argv[1:]
sys.argv = [str(pathlib.Path(root) / f"{site}_app.py"), era, f"--port={port}"]
spec = importlib.util.spec_from_file_location(f"{site}_app", sys.argv[0])
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)
module.load_or_create_index()
module.app.run(host="0.0.0.0", port=int(port), debug=False, use_reloader=False)
