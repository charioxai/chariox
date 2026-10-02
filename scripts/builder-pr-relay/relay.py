"""Mac-only Git/PR publisher. Builder input is data, never shell code."""
import argparse
import fcntl
from datetime import datetime, timezone
from urllib.parse import urlencode
import json
import os
import re
import shlex
import subprocess
import time
from pathlib import Path
from bridge import REPOS, FOOTER, valid_branch

TRAILER = "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
REF = re.compile(r"[A-Za-z0-9][A-Za-z0-9._/-]*\Z")


def validate_request(request):
    branch, base = request["branch"], request["base"]
    if not valid_branch(branch) or not isinstance(base, str) or not REF.fullmatch(base):
        raise ValueError("invalid head or base")
    if any(x in base for x in ("..", "//", "@{")) or any(not p or p.startswith(".") or p.endswith((".lock", ".")) for p in base.split("/")):
        raise ValueError("invalid base")
    if branch == base:
        raise ValueError("head equals base")
    if not isinstance(request["title"], str) or not request["title"].strip() or len(request["title"]) > 256:
        raise ValueError("invalid title")
    if not isinstance(request.get("draft", False), bool):
        raise ValueError("invalid draft flag")
    reply = request.get("reply")
    if reply is not None:
        if (not isinstance(reply, dict) or not re.fullmatch(r"[0-9a-f]{40}", reply.get("commit", ""))
                or not isinstance(reply.get("body"), str) or len(reply["body"]) > 8192
                or not reply["body"].startswith(f"Addressed in {reply['commit']}. ")):
            raise ValueError("invalid review reply")
    if not request["body"].rstrip().endswith(FOOTER):
        raise ValueError("missing PR footer")


def validate_commits(messages):
    for message in messages:
        lines = message.strip().splitlines()
        if not lines or "[skip ci]" not in lines[0] or lines[-1] != TRAILER:
            raise ValueError("commit requires [skip ci] and co-author trailer")


def choose_pr(pulls, branch):
    matches = [p for p in pulls if p["headRefName"] == branch]
    opened = [p for p in matches if p["state"] == "OPEN"]
    return max(opened or matches, key=lambda p: p["number"], default=None)


def run(argv, *, data=None, env=None):
    process = subprocess.run(argv, input=data, text=True, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, env=env, timeout=120)
    if process.returncode:
        # Avoid dumping tool diagnostics, which can contain credential-helper data.
        raise RuntimeError(f"{Path(argv[0]).name} failed with exit {process.returncode}")
    return process.stdout


class Relay:
    def __init__(self, args):
        self.args = args
        self.records = []
        self.failures = []
        self.env = dict(os.environ, GIT_TERMINAL_PROMPT="0", GH_PROMPT_DISABLED="1")
        self.env["GIT_SSH_COMMAND"] = shlex.join(["ssh", "-F", str(args.ssh_config), "-o", "BatchMode=yes", "-o", "ConnectTimeout=15"])

    def remote(self, action, data=None):
        return run(["ssh", "-F", str(self.args.ssh_config), "-o", "BatchMode=yes", "-o", "ConnectTimeout=15",
                    "p1b", f"python3 /root/.chariox/dev/builder-pr-relay/bridge.py {action}"], data=data)

    def git(self, repo, *arguments):
        return run(["git", "--git-dir", str(self.args.git_root / f"{repo}.git"), *arguments], env=self.env).strip()

    def gh(self, repo, *arguments, data=None):
        return run(["gh", *arguments, "-R", f"charioxai/{repo}"], data=data, env=self.env)

    def api(self, repo, suffix):
        return json.loads(run(["gh", "api", f"repos/charioxai/{repo}/{suffix}", "--paginate", "--slurp"], env=self.env))

    def refs(self, repo, prefix):
        result = self.git(repo, "for-each-ref", "--format=%(refname) %(objectname)", prefix)
        return {line.split()[0][len(prefix):]: line.split()[1] for line in result.splitlines()}

    def publish(self, repo, branch, head, existing, origin):
        if not valid_branch(branch):
            raise ValueError("invalid branch")
        if existing == head:
            return
        if existing:
            # Explicit ancestry check before a compare-and-swap push; never force divergence.
            self.git(repo, "merge-base", "--is-ancestor", existing, head)
        baseline = existing or origin.get("main")
        if not baseline:
            raise ValueError("missing GitHub baseline")
        commits = self.git(repo, "rev-list", head, "--not", baseline, *origin.values()).splitlines()
        validate_commits([self.git(repo, "show", "-s", "--format=%B", commit) for commit in commits])
        validate_commits([self.git(repo, "show", "-s", "--format=%B", head)])
        self.git(repo, "push", "--porcelain", f"--force-with-lease=refs/heads/{branch}:{existing or ''}",
                 "origin", f"{head}:refs/heads/{branch}")
        print(f"published {repo} {branch} {head}", flush=True)

    def reply(self, repo, pull, request, head):
        reply = request.get("reply")
        if not reply or pull["state"] != "OPEN":
            return
        if reply["commit"] != head:
            raise ValueError("review reply is not for published head")
        marker = f"<!-- builder-pr-relay:{head} -->"
        receipt = self.args.state_dir / f"reply-{repo}-{pull['number']}-{head}"
        if receipt.exists():
            return
        comments = [c for page in self.api(repo, f"issues/{pull['number']}/comments?per_page=100") for c in page]
        if not any(marker in c["body"] for c in comments):
            self.gh(repo, "pr", "comment", str(pull["number"]), "--body-file", "-", data=reply["body"] + "\n\n" + marker)
        receipt.write_text("posted\n")

    def acknowledge(self, repo, branch, value):
        self.records.append({"repo": repo, "key": f"branches/{branch}", "value": value})

    def repository(self, repo, requests):
        cache = self.args.git_root / f"{repo}.git"
        if not cache.exists():
            run(["git", "init", "--bare", str(cache)])
            self.git(repo, "remote", "add", "origin", f"https://github.com/charioxai/{repo}.git")
        self.git(repo, "fetch", "--prune", "origin", "+refs/heads/*:refs/remotes/origin/*")
        self.git(repo, "fetch", "--prune", "ssh://p1b/srv/" + repo + ".git", "+refs/heads/apps/p1-*:refs/builder/apps/p1-*")
        heads, origin = self.refs(repo, "refs/builder/"), self.refs(repo, "refs/remotes/origin/")
        pulls = json.loads(self.gh(repo, "pr", "list", "--state", "open", "--limit", "1000",
                                 "--json", "number,url,state,headRefName,headRefOid,reviews,comments"))
        if len(pulls) >= 1000:
            raise ValueError("open PR listing reached limit")
        by_branch = {r["branch"]: r for r in requests}
        for branch, head in heads.items():
            try:
                self.publish(repo, branch, head, origin.get(branch), origin)
                request = by_branch.get(branch)
                if not request:
                    continue
                validate_request(request)
                history = json.loads(self.gh(repo, "pr", "list", "--head", branch, "--state", "all", "--limit", "1000",
                                             "--json", "number,url,state,headRefName,headRefOid"))
                pull = choose_pr(history, branch)
                if pull is None:
                    options = ["pr", "create", "--head", branch, "--base", request["base"], "--title", request["title"], "--body-file", "-"]
                    if request.get("draft", False):
                        options.append("--draft")
                    url = self.gh(repo, *options, data=request["body"]).strip()
                    pull = json.loads(self.gh(repo, "pr", "view", url, "--json", "number,url,state,headRefName,headRefOid"))
                    pulls.append(pull)
                    print(f"opened {repo} PR {pull['number']}", flush=True)
                self.reply(repo, pull, request, head)
                self.acknowledge(repo, branch, {**pull, "published_sha": head, "status": "published"})
            except (ValueError, RuntimeError, KeyError, TypeError, subprocess.TimeoutExpired):
                self.failures.append(f"{repo} {branch}: publish/request rejected")
                if branch in by_branch:
                    self.acknowledge(repo, branch, {"status": "blocked", "head_sha": head, "error": "publish/request rejected; inspect Mac relay"})
        for request in requests:
            if request["branch"] not in heads:
                self.acknowledge(repo, request["branch"], {"status": "blocked", "error": "branch absent from builder mirror"})
        cache_path = self.args.state_dir / f"{repo}-inline.json"
        cached = json.loads(cache_path.read_text()) if cache_path.exists() else {"since": None, "comments": {}}
        started = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        query = {"per_page": 100}
        if cached["since"] and time.time() - cached.get("full_at", 0) < 86400:
            query["since"] = cached["since"]
        else:
            cached["comments"] = {}
            cached["full_at"] = time.time()
        for page in self.api(repo, "pulls/comments?" + urlencode(query)):
            for comment in page:
                cached["comments"][str(comment["id"])] = comment
        cached["since"] = started
        temporary = cache_path.with_suffix(".tmp")
        temporary.write_text(json.dumps(cached))
        temporary.replace(cache_path)
        inline = {}
        for comment in cached["comments"].values():
            number = int(comment["pull_request_url"].rsplit("/", 1)[1])
            inline.setdefault(number, []).append(comment)
        for pull in pulls:
            if pull["state"] != "OPEN":
                continue
            # Creation returns a small projection; obtain feedback on the next bulk tick.
            if "reviews" not in pull:
                pull.update(json.loads(self.gh(repo, "pr", "view", str(pull["number"]), "--json", "reviews,comments")))
            value = dict(pull, mirrored_at=int(time.time()))
            value["reviews"] = [{"author": (r.get("author") or {}).get("login"), "state": r.get("state"),
                                  "body": r.get("body"), "commit_id": (r.get("commit") or {}).get("oid")}
                                 for r in value["reviews"]]
            value["comments"] = [{"author": (c.get("author") or {}).get("login"), "state": "COMMENTED",
                                   "body": c.get("body"), "commit_id": None} for c in value["comments"]]
            value["comments"].extend({"author": (c.get("user") or {}).get("login"), "state": "COMMENTED",
                                      "body": c.get("body"), "commit_id": c.get("commit_id"),
                                      "path": c.get("path"), "line": c.get("line")} for c in inline.get(pull["number"], []))
            self.records.append({"repo": repo, "key": str(pull["number"]), "value": value})

    def tick(self):
        self.records, self.failures = [], []
        inbox = json.loads(self.remote("read"))
        for error in inbox["errors"]:
            self.failures.append(f"{error['repo']} invalid request")
        for repo in REPOS:
            try:
                self.repository(repo, inbox["requests"][repo])
            except (RuntimeError, ValueError, subprocess.TimeoutExpired):
                self.failures.append(f"{repo}: repository unavailable")
        self.remote("write", json.dumps(self.records))
        report = {"at": int(time.time()), "mirrored_records": len(self.records), "failures": self.failures}
        temporary = self.args.state_dir / "status.tmp"
        temporary.write_text(json.dumps(report, indent=2) + "\n")
        temporary.replace(self.args.state_dir / "status.json")
        print(f"tick mirrored={len(self.records)} blocked={len(self.failures)}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ssh-config", type=Path, required=True)
    parser.add_argument("--git-root", type=Path, required=True)
    parser.add_argument("--state-dir", type=Path, required=True)
    parser.add_argument("--loop", action="store_true", help="poll every 90 seconds; default is one tick")
    args = parser.parse_args()
    args.state_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    args.git_root.mkdir(parents=True, exist_ok=True, mode=0o700)
    with (args.state_dir / "lock").open("w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        (args.state_dir / "pid").write_text(str(os.getpid()) + "\n")
        relay = Relay(args)
        while True:
            try:
                relay.tick()
            except (RuntimeError, ValueError, subprocess.TimeoutExpired):
                print("tick failed; retry on next interval", flush=True)
                if not args.loop:
                    raise
            if not args.loop:
                break
            time.sleep(90)


if __name__ == "__main__":
    main()
