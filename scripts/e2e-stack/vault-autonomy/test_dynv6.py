"""MP-08 / MP-09 / MP-10 / MP-11: supplementary signup/cleanup regressions."""

import importlib.util
import io
import json
from pathlib import Path
import stat
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "dynv6", Path(__file__).with_name("dynv6.py")
)
client = importlib.util.module_from_spec(spec)
spec.loader.exec_module(client)


class SignupTest(unittest.TestCase):
    def test_vue_key_and_account_components_are_observed(self):
        page = client.Page(
            '<b-dropdown-item href="/users/sign_out" data-method="delete">Sign out</b-dropdown-item>'
            '<key-data :k=\'{"id":1,"data":"synthetic-key","regen":false}\'></key-data>'
            '<credential-settings account-url="/users"></credential-settings>'
        )
        self.assertTrue(client.authenticated_page(page))
        self.assertEqual(page.key_values, ["synthetic-key"])
        self.assertEqual(page.account_url, "/users")

    def test_confirmation_status_comes_from_service_account_metadata(self):
        pending = client.Page(
            "<email-form attributes='{\"confirmed_at\":null}'></email-form>"
        )
        confirmed = client.Page(
            '<email-form attributes=\'{"confirmed_at":"2026-11-02"}\'></email-form>'
        )
        self.assertFalse(pending.email_confirmed)
        self.assertTrue(confirmed.email_confirmed)

    def test_authenticated_signup_does_not_require_confirmation_email(self):
        result = client.signup_result(
            200,
            "https://dynv6.com/zones/new",
            '<a href="/users/sign_out" data-method="delete">Sign out</a>',
        )
        self.assertTrue(result["registered"])
        self.assertTrue(result["authenticated"])
        self.assertFalse(result["confirmation_required"])

    def test_authenticated_pending_email_page_is_signup_success(self):
        result = client.signup_result(
            200,
            "https://dynv6.com/zones/new",
            '<a href="/users/edit">Account</a>Please confirm your email',
        )
        self.assertTrue(result["registered"])
        self.assertTrue(result["authenticated"])
        self.assertTrue(result["confirmation_required"])

    def test_redirect_path_alone_is_not_success(self):
        self.assertFalse(
            client.signup_result(200, "https://dynv6.com/zones/new", "Please sign in")[
                "registered"
            ]
        )

    def test_script_confirmation_text_is_not_a_service_notice(self):
        self.assertFalse(
            client.signup_result(
                200,
                "https://dynv6.com/users/sign_in",
                '<script>const label = "confirm your email";</script>Please sign in',
            )["registered"]
        )

    def test_confirmation_flow_remains_supported(self):
        result = client.signup_result(
            200,
            "https://dynv6.com/users/sign_in",
            "A confirmation link has been sent to your email address.",
        )
        self.assertTrue(result["registered"])
        self.assertTrue(result["confirmation_required"])
        self.assertFalse(result["authenticated"])

    def test_service_error_is_not_success(self):
        self.assertFalse(
            client.signup_result(
                422, "https://dynv6.com/users", "Email has already been taken"
            )["registered"]
        )

    def test_response_loss_keeps_private_cleanup_access(self):
        with tempfile.TemporaryDirectory() as directory:
            client.STATE = Path(directory) / "private" / "account.json"
            auth = {"email": "regression@example.invalid", "password": "test-value"}
            with patch.object(client, "fetch", side_effect=TimeoutError):
                with self.assertRaises(TimeoutError):
                    client.signup(auth)
            self.assertEqual(client.json.loads(client.STATE.read_text()), auth)
            self.assertEqual(stat.S_IMODE(client.STATE.stat().st_mode), 0o600)
            self.assertEqual(stat.S_IMODE(client.STATE.parent.stat().st_mode), 0o700)
            with self.assertRaises(FileExistsError):
                client.save_setup({"email": "replacement", "password": "replacement"})
            self.assertEqual(client.json.loads(client.STATE.read_text()), auth)


class CleanupTest(unittest.TestCase):
    def run_cleanup(self, second_login, cancel_result):
        page = '<credential-settings account-url="/users"></credential-settings>'
        response = io.BytesIO(json.dumps(cancel_result).encode())
        response.status = 200
        responses = [io.BytesIO(page.encode()), response, io.BytesIO(b"Cancelled")]
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "account.json"
            state.write_text(
                json.dumps(
                    {"email": "synthetic@example.invalid", "password": "synthetic"}
                )
            )
            with (
                patch.object(
                    client.sys,
                    "argv",
                    ["dynv6.py", "--state", str(state), "delete-account"],
                ),
                patch.object(client, "login", side_effect=[object(), second_login]),
                patch.object(client, "fetch", side_effect=responses),
                redirect_stdout(io.StringIO()) as output,
            ):
                try:
                    client.main()
                except Exception as error:
                    return state.exists(), type(error), json.loads(state.read_text())
                return state.exists(), None, json.loads(output.getvalue())

    def test_confirmed_deletion_and_refused_login_remove_private_setup(self):
        retained, error, result = self.run_cleanup(
            RuntimeError("service login did not complete"), {"location": "/"}
        )
        self.assertFalse(retained)
        self.assertIsNone(error)
        self.assertTrue(result["account_deleted"])
        self.assertTrue(result["service_cancellation_acknowledged"])

    def test_network_failure_after_cancellation_keeps_reconciliation_proof(self):
        retained, error, state = self.run_cleanup(TimeoutError(), {"location": "/"})
        self.assertTrue(retained)
        self.assertEqual(error, TimeoutError)
        self.assertTrue(state["deletion_acknowledged"])

    def test_unexpected_cancellation_result_keeps_private_setup(self):
        retained, error, state = self.run_cleanup(object(), {"location": "/users/edit"})
        self.assertTrue(retained)
        self.assertEqual(error, RuntimeError)
        self.assertNotIn("deletion_acknowledged", state)

    def test_cleanup_reconciles_previous_ack_without_deleting_again(self):
        with tempfile.TemporaryDirectory() as directory:
            client.STATE = Path(directory) / "account.json"
            client.STATE.write_text(json.dumps({"deletion_acknowledged": True}))
            with (
                patch.object(
                    client,
                    "login",
                    side_effect=RuntimeError("service login did not complete"),
                ),
                patch.object(client, "fetch") as fetch,
                redirect_stdout(io.StringIO()),
            ):
                client.delete_account()
                fetch.assert_not_called()
            self.assertFalse(client.STATE.exists())


if __name__ == "__main__":
    unittest.main()
