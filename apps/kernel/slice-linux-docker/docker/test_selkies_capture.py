import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("selkies_capture", Path(__file__).with_name("selkies-capture.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class CaptureLifetimeTests(unittest.IsolatedAsyncioTestCase):
    def server(self, consumers, display_clients=None):
        class Server:
            async def _reconfigure_displays_locked(self):
                self.original_calls += 1
            def _active_primary_consumers(self):
                return consumers
            async def _stop_capture_for_display(self, display_id):
                self.stopped.append(display_id)
                del self.capture_instances[display_id]
        module.retain_primary_for_viewers(Server)
        server = Server()
        server.display_clients = display_clients or {}
        server.capture_instances = {"primary": object(), "secondary": object()}
        server.original_calls = 0
        server.stopped = []
        return server

    async def test_live_readonly_viewer_retains_primary_without_display_ownership(self):
        server = self.server({"viewer"})
        await server._reconfigure_displays_locked()
        self.assertEqual(server.original_calls, 0)
        self.assertEqual(server.stopped, ["secondary"])
        self.assertIn("primary", server.capture_instances)
        self.assertEqual(server.display_clients, {})

    async def test_last_or_paused_viewer_keeps_upstream_teardown(self):
        server = self.server(set())
        await server._reconfigure_displays_locked()
        self.assertEqual(server.original_calls, 1)

    async def test_display_owned_layout_keeps_upstream_reconfiguration(self):
        server = self.server({"viewer"}, {"primary": {"ws": "controller"}})
        await server._reconfigure_displays_locked()
        self.assertEqual(server.original_calls, 1)


if __name__ == "__main__":
    unittest.main()
