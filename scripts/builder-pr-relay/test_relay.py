import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from bridge import collect, write_mirror, valid_branch, FOOTER
from relay import Relay, choose_pr, validate_commits, validate_request, TRAILER


class RelayTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def request(self, **overrides):
        return dict(branch="apps/p1-proof", base="main", title="proof", body="proof\n\n" + FOOTER, **overrides)

    def test_ref_and_flag_injection_rejected(self):
        for value in ("--all", "main", "apps/p1-../x", "apps/p1-x//y", "apps/p1-x.lock", "apps/p1-x@{x", "apps/p1-x;touch x"):
            self.assertFalse(valid_branch(value), value)
        self.assertTrue(valid_branch("apps/p1-proof/nested"))
        request = self.request()
        request["base"] = "--delete"
        with self.assertRaises(ValueError):
            validate_request(request)

    def test_commit_range_requires_skip_ci_and_trailer(self):
        good = "fix [skip ci]\n\n" + TRAILER
        validate_commits([good])
        for message in ("fix\n\n" + TRAILER, "fix [skip ci]", "fix\n\n[skip ci]\n" + TRAILER):
            with self.assertRaises(ValueError):
                validate_commits([good, message])

    def test_closed_pr_is_not_recreated_and_open_pr_wins(self):
        closed = dict(number=3, headRefName="apps/p1-proof", state="CLOSED")
        opened = dict(number=2, headRefName="apps/p1-proof", state="OPEN")
        self.assertEqual(choose_pr([closed], "apps/p1-proof"), closed)
        self.assertEqual(choose_pr([closed, opened], "apps/p1-proof"), opened)
        self.assertIsNone(choose_pr([closed], "apps/p1-other"))

    def make_inbox(self):
        directory = self.root / "chariox/apps"
        directory.mkdir(parents=True)
        body = self.root / "chariox/body.md"
        body.write_text("proof\n\n" + FOOTER)
        request = dict(base="main", title="proof", body_file=str(body), draft=True)
        (directory / "p1-proof.json").write_text(json.dumps(request))
        return body, directory

    def test_request_reads_body_without_running_it(self):
        body, directory = self.make_inbox()
        body.write_text("$(touch forbidden) `echo hidden`\n\n" + FOOTER)
        result = collect(self.root)
        self.assertEqual(result["errors"], [])
        request = result["requests"]["chariox"][0]
        validate_request(request)
        self.assertEqual(request["branch"], "apps/p1-proof")
        self.assertIn("$(touch forbidden)", request["body"])

    def test_external_body_and_symlink_rejected(self):
        body, directory = self.make_inbox()
        outside = self.root.parent / "outside.md"
        request = json.loads((directory / "p1-proof.json").read_text())
        request["body_file"] = str(outside)
        (directory / "p1-proof.json").write_text(json.dumps(request))
        self.assertEqual(len(collect(self.root)["errors"]), 1)
        request["body_file"] = str(body)
        (directory / "p1-proof.json").write_text(json.dumps(request))
        body.unlink()
        body.symlink_to("/etc/passwd")
        self.assertEqual(len(collect(self.root)["errors"]), 1)

    def test_fifo_does_not_block(self):
        body, directory = self.make_inbox()
        body.unlink()
        os.mkfifo(body)
        self.assertEqual(len(collect(self.root)["errors"]), 1)

    def test_atomic_mirror_and_symlink_parent(self):
        write_mirror(self.root, "chariox/12.json", {"reviews": []})
        self.assertEqual(json.loads((self.root / "chariox/12.json").read_text()), {"reviews": []})
        self.assertEqual((self.root / "chariox/12.json").stat().st_mode & 0o777, 0o600)
        (self.root / "chariox/link").symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(OSError):
            write_mirror(self.root, "chariox/link/evil.json", {})

    def test_invalid_draft_and_missing_footer(self):
        request = self.request(draft="false")
        with self.assertRaises(ValueError):
            validate_request(request)
        request = self.request()
        request["body"] = "no footer"
        with self.assertRaises(ValueError):
            validate_request(request)


class GitPublicationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source"
        self.remote = self.root / "remote.git"
        self.cache = self.root / "chariox.git"
        self.command("git", "init", "-b", "main", str(self.source))
        self.command("git", "-C", str(self.source), "config", "user.name", "test")
        self.command("git", "-C", str(self.source), "config", "user.email", "test@example.com")
        self.base = self.commit("base")
        self.command("git", "clone", "--bare", str(self.source), str(self.remote))
        self.command("git", "clone", "--bare", str(self.source), str(self.cache))
        self.command("git", "--git-dir", str(self.cache), "remote", "set-url", "origin", str(self.remote))
        self.relay = Relay(SimpleNamespace(ssh_config=self.root / "unused", git_root=self.root))

    def command(self, *argv):
        return subprocess.run(argv, text=True, capture_output=True, check=True).stdout.strip()

    def commit(self, subject, trailer=True):
        message = subject + ("\n\n" + TRAILER if trailer else "")
        self.command("git", "-C", str(self.source), "commit", "--allow-empty", "-m", message)
        return self.command("git", "-C", str(self.source), "rev-parse", "HEAD")

    def fetch(self):
        self.command("git", "--git-dir", str(self.cache), "fetch", str(self.source), "HEAD")

    def test_new_branch_and_fast_forward_publish(self):
        head = self.commit("one [skip ci]")
        self.fetch()
        self.relay.publish("chariox", "apps/p1-proof", head, None, {"main": self.base})
        self.assertEqual(self.command("git", "--git-dir", str(self.remote), "rev-parse", "apps/p1-proof"), head)
        next_head = self.commit("two [skip ci]")
        self.fetch()
        self.relay.publish("chariox", "apps/p1-proof", next_head, head, {"main": self.base, "apps/p1-proof": head})
        self.assertEqual(self.command("git", "--git-dir", str(self.remote), "rev-parse", "apps/p1-proof"), next_head)

    def test_hidden_bad_commit_blocks_entire_range(self):
        self.commit("would trigger CI")
        head = self.commit("valid head [skip ci]")
        self.fetch()
        with self.assertRaises(ValueError):
            self.relay.publish("chariox", "apps/p1-proof", head, None, {"main": self.base})
        self.assertNotIn("apps/p1-proof", self.command("git", "--git-dir", str(self.remote), "for-each-ref", "--format=%(refname)"))

    def test_divergence_and_racing_remote_never_overwritten(self):
        head = self.commit("one [skip ci]")
        self.fetch()
        self.relay.publish("chariox", "apps/p1-proof", head, None, {"main": self.base})
        self.command("git", "-C", str(self.source), "reset", "--hard", self.base)
        divergent = self.commit("divergent [skip ci]")
        self.fetch()
        with self.assertRaises(RuntimeError):
            self.relay.publish("chariox", "apps/p1-proof", divergent, head, {"main": self.base})
        with self.assertRaises(RuntimeError):
            self.relay.publish("chariox", "apps/p1-proof", divergent, None, {"main": self.base})
        self.assertEqual(self.command("git", "--git-dir", str(self.remote), "rev-parse", "apps/p1-proof"), head)


class PublicationFlowTests(unittest.TestCase):
    def test_create_retry_reply_and_feedback(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "chariox.git").mkdir()
            args = SimpleNamespace(ssh_config=root / "unused", git_root=root, state_dir=root)
            sha = "a" * 40
            review = dict(author={"login": "reviewer"}, state="COMMENTED", body="finding", commit={"oid": sha})
            comment = dict(author={"login": "owner"}, body="issue comment")
            class FakeRelay(Relay):
                created = 0
                commented = 0
                closed = False
                marker = None
                def git(self, repo, *arguments):
                    return ""
                def refs(self, repo, prefix):
                    return {"apps/p1-proof": sha} if prefix == "refs/builder/" else {"main": "b" * 40}
                def publish(self, *arguments):
                    pass
                def gh(self, repo, *arguments, data=None):
                    pull = dict(number=10, url="https://example.test/10", state="CLOSED" if self.closed else "OPEN",
                                headRefName="apps/p1-proof", headRefOid=sha, reviews=[review], comments=[comment])
                    if arguments[:2] == ("pr", "list"):
                        return json.dumps([pull] if self.created else [])
                    if arguments[:2] == ("pr", "create"):
                        self.created += 1
                        return pull["url"]
                    if arguments[:2] == ("pr", "comment"):
                        self.commented += 1
                        self.marker = data
                        return ""
                    return json.dumps(pull)
                def api(self, repo, suffix):
                    if suffix.startswith("issues/"):
                        return [[dict(body=self.marker)]] if self.marker else [[]]
                    return [[dict(id=1, pull_request_url="https://example.test/pulls/10", user={"login": "reviewer"},
                                  body="inline finding", commit_id=sha, path="file", line=1)]]
            relay = FakeRelay(args)
            request = dict(branch="apps/p1-proof", base="main", title="proof", body=FOOTER, draft=True,
                           reply=dict(commit=sha, body=f"Addressed in {sha}. Fixed it."))
            relay.repository("chariox", [request])
            self.assertEqual((relay.created, relay.commented), (1, 1))
            mirror = [x["value"] for x in relay.records if x["key"] == "10"][0]
            self.assertEqual(mirror["reviews"][0]["commit_id"], sha)
            self.assertIsNone(mirror["comments"][0]["commit_id"])
            self.assertEqual(mirror["comments"][1]["commit_id"], sha)
            relay.records = []
            relay.repository("chariox", [request])
            self.assertEqual((relay.created, relay.commented), (1, 1))
            # Simulate lost local receipt; remote marker still prevents duplicate reply.
            (root / f"reply-chariox-10-{sha}").unlink()
            relay.repository("chariox", [request])
            self.assertEqual(relay.commented, 1)
            relay.closed = True
            relay.records = []
            relay.repository("chariox", [request])
            self.assertEqual(relay.created, 1)
            self.assertEqual(relay.records[0]["value"]["state"], "CLOSED")

    def test_review_reply_must_match_full_sha(self):
        request = dict(branch="apps/p1-proof", base="main", title="proof", body=FOOTER,
                       reply=dict(commit="abc", body="Addressed in abc. Done."))
        with self.assertRaises(ValueError):
            validate_request(request)


if __name__ == "__main__":
    unittest.main()
