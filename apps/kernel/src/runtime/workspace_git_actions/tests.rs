use super::*;
use crate::runtime::bounded_process::{with_test_command_limit, with_test_mutation_limit};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const TOKEN: &str = "ghp_sentinelsecret0123456789";

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = crate::process_spawn::command("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

struct Fixture {
    root: PathBuf,
    clone: PathBuf,
    remote: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-git-{label}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let remote = root.join("remote.git");
        let clone = root.join("clone");
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "--bare", "-b", "main", "remote.git"]);
        git(&root, &["clone", "remote.git", "clone"]);
        git(&clone, &["config", "user.email", "tests@example.invalid"]);
        git(&clone, &["config", "user.name", "Chariox Tests"]);
        std::fs::write(clone.join("README.md"), "seed\n").unwrap();
        git(&clone, &["add", "."]);
        git(&clone, &["commit", "-m", "seed"]);
        git(&clone, &["push", "-u", "origin", "HEAD:main"]);
        git(&clone, &["branch", "--set-upstream-to=origin/main"]);
        Self {
            root,
            clone,
            remote,
        }
    }

    fn path(&self) -> &str {
        self.clone.to_str().unwrap()
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.clone.join(name), text).unwrap();
    }

    fn commit(&self, files: &[&str], message: &str, branch: Option<&str>) -> CommitOutcome {
        let files: Vec<String> = files.iter().map(ToString::to_string).collect();
        commit_workspace_changes("w", self.path(), &files, message, branch).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_commit_holds_exactly_the_named_files_even_when_others_are_staged_or_changed() {
    let fixture = Fixture::new("exact");
    fixture.write("wanted.txt", "wanted\n");
    fixture.write("staged.txt", "staged\n");
    fixture.write("loose.txt", "loose\n");
    git(&fixture.clone, &["add", "staged.txt"]);

    let outcome = fixture.commit(&["wanted.txt"], "Add wanted", None);

    let committed = git(
        &fixture.clone,
        &["show", "--name-only", "--format=", "HEAD"],
    );
    assert_eq!(committed, "wanted.txt");
    let left = git(&fixture.clone, &["status", "--porcelain"]);
    assert!(
        left.contains("staged.txt") && left.contains("loose.txt"),
        "{left}"
    );
    assert_eq!(
        outcome.result.commit_sha,
        Some(git(&fixture.clone, &["rev-parse", "HEAD"]))
    );
}

#[test]
fn work_on_the_default_branch_moves_to_a_feature_branch_and_a_feature_branch_stays() {
    let fixture = Fixture::new("branch");
    fixture.write("a.txt", "a\n");
    let first = fixture.commit(&["a.txt"], "Add the A file!", None);
    assert!(first.created_branch);
    assert_eq!(
        first.result.branch.as_deref(),
        Some("chariox/add-the-a-file")
    );
    assert_eq!(
        git(&fixture.clone, &["rev-parse", "main"]),
        git(&fixture.clone, &["rev-parse", "origin/main"])
    );

    fixture.write("b.txt", "b\n");
    let second = fixture.commit(&["b.txt"], "Add the B file", None);
    assert!(!second.created_branch);
    assert_eq!(
        second.result.branch.as_deref(),
        Some("chariox/add-the-a-file")
    );

    git(&fixture.clone, &["switch", "main"]);
    fixture.write("c.txt", "c\n");
    let third = fixture.commit(&["c.txt"], "Add the A file!", None);
    assert_eq!(
        third.result.branch.as_deref(),
        Some("chariox/add-the-a-file-2")
    );
}

#[test]
fn a_commit_names_changed_files_a_message_and_never_the_default_branch() {
    let fixture = Fixture::new("refusals");
    fixture.write("a.txt", "a\n");
    let attempt = |files: &[&str], message: &str, branch: Option<&str>| {
        let files: Vec<String> = files.iter().map(ToString::to_string).collect();
        commit_workspace_changes("w", fixture.path(), &files, message, branch)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default()
    };
    assert!(attempt(&[], "m", None).contains("between 1 and 500"));
    assert!(attempt(&["README.md"], "m", None).contains("not a changed file"));
    assert!(attempt(&["../outside"], "m", None).contains("not a changed file"));
    assert!(attempt(&["a.txt"], "  ", None).contains("message is required"));
    assert!(attempt(&["a.txt"], "m", Some("main")).contains("not a usable feature branch"));
    assert!(attempt(&["a.txt"], "m", Some("-bad")).contains("not a usable feature branch"));
    assert_eq!(
        git(&fixture.clone, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main"
    );
}

#[test]
fn a_named_branch_is_created_then_reused() {
    let fixture = Fixture::new("named");
    fixture.write("a.txt", "a\n");
    let first = fixture.commit(&["a.txt"], "one", Some("topic/x"));
    assert!(first.created_branch);
    fixture.write("b.txt", "b\n");
    let second = fixture.commit(&["b.txt"], "two", Some("topic/x"));
    assert!(!second.created_branch);
    assert_eq!(second.result.branch.as_deref(), Some("topic/x"));
}

#[test]
fn the_default_branch_is_never_pushed_and_a_push_needs_a_github_https_origin() {
    let fixture = Fixture::new("guards");
    let credential = PushCredential::from_token(TOKEN);
    let error = push_workspace_branch("w", fixture.path(), &credential)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("https://github.com/<owner>/<repo>"),
        "{error}"
    );

    git(
        &fixture.clone,
        &["remote", "set-url", "origin", "https://github.com/o/r.git"],
    );
    let error = push_branch("w", fixture.path(), &[], &[])
        .unwrap_err()
        .to_string();
    assert!(error.contains("default branch"), "{error}");
    git(&fixture.clone, &["checkout", "--detach"]);
    let error = push_branch("w", fixture.path(), &[], &[])
        .unwrap_err()
        .to_string();
    assert!(error.contains("detached HEAD"), "{error}");
}

#[test]
fn a_feature_branch_pushes_with_an_upstream_and_never_forces() {
    let fixture = Fixture::new("push");
    fixture.write("a.txt", "a\n");
    fixture.commit(&["a.txt"], "Add a", None);
    let pushed = push_branch("w", fixture.path(), &[], &[]).unwrap();
    assert_eq!(pushed.branch.as_deref(), Some("chariox/add-a"));
    assert_eq!(
        git(&fixture.remote, &["rev-parse", "chariox/add-a"]),
        git(&fixture.clone, &["rev-parse", "HEAD"])
    );
    assert_eq!(
        git(&fixture.clone, &["rev-parse", "--abbrev-ref", "@{u}"]),
        "origin/chariox/add-a"
    );

    // The remote moved on: a plain push is refused, not forced over.
    git(&fixture.clone, &["commit", "--amend", "-m", "Rewritten"]);
    let error = push_branch("w", fixture.path(), &[], &[])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("rejected") || error.contains("non-fast-forward"),
        "{error}"
    );
}

#[test]
fn the_credential_reaches_git_by_environment_only_and_is_scrubbed_from_its_failure() {
    let credential = PushCredential::from_token(TOKEN);
    let environment = credential.environment();
    let header = environment
        .iter()
        .find(|(name, _)| name == "GIT_CONFIG_VALUE_1")
        .map(|(_, value)| value.clone())
        .unwrap();
    let basic = header.strip_prefix("Authorization: Basic ").unwrap();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(basic)
            .unwrap(),
        format!("x-access-token:{TOKEN}").into_bytes()
    );
    assert!(environment
        .iter()
        .any(|(name, value)| name == "GIT_CONFIG_KEY_0" && value == "credential.helper"));

    // A remote hook that prints the header git was given must not get it back.
    let fixture = Fixture::new("scrub");
    fixture.write("a.txt", "a\n");
    fixture.commit(&["a.txt"], "Add a", None);
    let hook = fixture.remote.join("hooks/pre-receive");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(
        &hook,
        "#!/bin/sh\necho \"got $GIT_CONFIG_VALUE_1\" >&2\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let error = push_branch("w", fixture.path(), &environment, &[basic])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("got Authorization: Basic [redacted]"),
        "{error}"
    );
    assert!(!error.contains(basic) && !error.contains(TOKEN), "{error}");
}

#[test]
fn only_github_https_remotes_are_repositories() {
    for (url, expected) in [
        (
            "https://github.com/charioxai/chariox.git",
            Some("charioxai/chariox"),
        ),
        ("https://github.com/o/r", Some("o/r")),
        ("https://github.com/o/r/", Some("o/r")),
        ("git@github.com:o/r.git", None),
        ("https://gitlab.com/o/r.git", None),
        ("https://github.com.evil.test/o/r.git", None),
        ("https://github.com/o", None),
        ("/tmp/remote.git", None),
    ] {
        assert_eq!(github_https_repository(url).as_deref(), expected, "{url}");
    }
}

#[cfg(unix)]
mod slow_hooks {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    const QUERY_CAP: Duration = Duration::from_secs(1);

    fn install_hook(repo_git_dir: &Path, name: &str, body: &str) {
        let hook = repo_git_dir.join("hooks").join(name);
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(&hook, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn commit_with_a_hook_slower_than_the_query_cap_succeeds() {
        let fixture = Fixture::new("slow-commit");
        fixture.write("change.txt", "change\n");
        install_hook(&fixture.clone.join(".git"), "pre-commit", "sleep 3");
        let started = Instant::now();
        let result = with_test_command_limit(QUERY_CAP, || {
            commit_workspace_changes(
                "w",
                fixture.path(),
                &["change.txt".into()],
                "slow hook",
                None,
            )
        });
        assert!(result.is_ok(), "commit failed: {:?}", result.err());
        assert!(started.elapsed() >= Duration::from_secs(3));
    }

    #[test]
    fn push_with_a_remote_hook_slower_than_the_query_cap_succeeds() {
        let fixture = Fixture::new("slow-push");
        fixture.write("change.txt", "change\n");
        fixture.commit(&["change.txt"], "to push", None);
        install_hook(&fixture.remote, "pre-receive", "sleep 3");
        let result =
            with_test_command_limit(QUERY_CAP, || push_branch("w", fixture.path(), &[], &[]));
        assert!(result.is_ok(), "push failed: {:?}", result.err());
    }

    fn process_is_alive(pid: i32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|stat| !stat.contains(") Z "))
            .unwrap_or(false)
    }

    #[test]
    fn hung_commit_hook_ends_at_the_mutation_deadline_with_its_tree_reaped() {
        let fixture = Fixture::new("hung-commit");
        fixture.write("change.txt", "change\n");
        let pid_file = fixture.root.join("hook.pid");
        install_hook(
            &fixture.clone.join(".git"),
            "pre-commit",
            &format!("sleep 300 &\necho $! > '{}'\nwait", pid_file.display()),
        );
        let started = Instant::now();
        let error = with_test_mutation_limit(Duration::from_secs(2), || {
            commit_workspace_changes(
                "w",
                fixture.path(),
                &["change.txt".into()],
                "hung hook",
                None,
            )
        })
        .err()
        .expect("a hung hook must end at the mutation deadline");
        assert!(started.elapsed() < Duration::from_secs(20), "{error}");
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .expect("hook should have started its child")
            .trim()
            .parse()
            .unwrap();
        let reaped_by = Instant::now() + Duration::from_secs(5);
        while process_is_alive(pid) && Instant::now() < reaped_by {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            !process_is_alive(pid),
            "hook child {pid} outlived the deadline"
        );
    }
}
