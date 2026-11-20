"""MP-08 / MP-10 / MP-11: only the direct CLI's expected refusal succeeds."""
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "outcome", Path(__file__).with_name("kernel_access_requester_outcome.py"))
outcome = importlib.util.module_from_spec(spec)
spec.loader.exec_module(outcome)

REFUSAL = json.dumps({"error": {
    "code": "kernel_access_denied",
    "message": outcome.REFUSAL_DIAGNOSTIC,
}})
CAPTURE = "Waiting for the owner to approve kernel access.\n" + REFUSAL + "\n"


class RequesterExit(unittest.TestCase):
    def test_direct_cli_expected_refusal(self):
        outcome.verify_requester_exit(3, CAPTURE, local_cli=True)

    def test_direct_cli_other_diagnostics_fail(self):
        for diagnostic in ["", "socket unavailable\n", "sudo request refused\n",
                           "prefix " + REFUSAL, REFUSAL + " unexpected suffix",
                           REFUSAL + "\nsocket unavailable\n"]:
            with self.subTest(diagnostic=diagnostic), self.assertRaises(RuntimeError):
                outcome.verify_requester_exit(3, diagnostic, local_cli=True)

    def test_direct_cli_unexpected_exits_fail_even_with_refusal(self):
        for code in [0, 1, 2, -15, None]:
            with self.subTest(code=code), self.assertRaises(RuntimeError):
                outcome.verify_requester_exit(code, CAPTURE, local_cli=True)

    def test_codex_success_is_independent_of_inner_cli_exit(self):
        outcome.verify_requester_exit(0, "", local_cli=False)
        outcome.verify_requester_exit(0, CAPTURE, local_cli=False)

    def test_codex_failure_is_not_hidden_by_refusal(self):
        for code in [1, 2, -15, None]:
            with self.subTest(code=code), self.assertRaises(RuntimeError):
                outcome.verify_requester_exit(code, CAPTURE, local_cli=False)


if __name__ == "__main__":
    unittest.main()
