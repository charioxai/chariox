"""MP-08 / MP-10: unchanged official SPA on slice loopback only."""
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import sys
from urllib.parse import urlsplit


root = Path(sys.argv[1]).resolve()


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(root), **kwargs)

    def do_GET(self):
        target = Path(self.translate_path(urlsplit(self.path).path)).resolve()
        if target.is_relative_to(root) and not target.exists():
            self.path = "/index.html"
        super().do_GET()

    def log_message(self, *args):
        pass


ThreadingHTTPServer(("127.0.0.1", 8765), Handler).serve_forever()
