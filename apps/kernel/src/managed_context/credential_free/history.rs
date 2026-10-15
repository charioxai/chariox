//! MP-08 / MP-11: one bounded Git stream inspects all objects, including
//! unreachable history and every tree name. Trees stream first, so each blob is
//! classified under every name it has. Limits are bytes/time, never counts.
use super::super::development::vault_artifacts::VaultItem;
use super::*;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

pub(crate) fn scan_repository(root: &Path) -> Result<Vec<VaultItem>, DaemonError> {
    #[cfg(test)]
    let scan_limit = tests::history_scan_limit();
    #[cfg(not(test))]
    let scan_limit = Duration::from_secs(120);
    let paths = git(root, &["rev-list", "--objects", "HEAD"], MAX_FILE)?;
    let paths = std::str::from_utf8(&paths)
        .map_err(|_| refused())?
        .lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(oid, path)| (oid.to_owned(), path.to_owned()))
        .collect::<BTreeMap<_, _>>();
    let listing = git(
        root,
        &[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objecttype) %(objectname)",
        ],
        MAX_FILE,
    )?;
    let listing = String::from_utf8(listing).map_err(|_| refused())?;
    let mut order = String::with_capacity(listing.len());
    for trees in [true, false] {
        for (kind, oid) in listing.lines().filter_map(|line| line.split_once(' ')) {
            if (kind == "tree") == trees {
                order.push_str(oid);
                order.push('\n');
            }
        }
    }
    let mut child = Command::new("git")
        .args(["cat-file", "--batch"])
        .current_dir(root)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| refused())?;
    #[cfg(test)]
    tests::interrupt_scan_child(&mut child, &["--batch"]);
    let stdout = child.stdout.take().ok_or_else(refused)?;
    let mut stdin = child.stdin.take().ok_or_else(refused)?;
    let writer = std::thread::spawn(move || {
        let _ = std::io::Write::write_all(&mut stdin, order.as_bytes());
    });
    let child = Arc::new(Mutex::new(child));
    let (done, deadline) = mpsc::channel::<()>();
    let watched = child.clone();
    let killed_for_refusal = Arc::new(AtomicBool::new(false));
    let watchdog_killed = killed_for_refusal.clone();
    let timed_out = Arc::new(AtomicBool::new(false));
    let watchdog_timeout = timed_out.clone();
    let watchdog = std::thread::spawn(move || {
        if matches!(
            deadline.recv_timeout(scan_limit),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            let mut owned = watched.lock().expect("owned Git child");
            if !watchdog_killed.load(Ordering::Relaxed) && owned.id() > 1 && owned.kill().is_ok() {
                watchdog_timeout.store(true, Ordering::Relaxed);
                watchdog_killed.store(true, Ordering::Relaxed);
            }
        }
    });
    let start = Instant::now();
    // MP-08 / MP-11: retain the pipe through policy kill and status collection.
    // Dropping it on refusal can turn a terminal limit into retryable SIGPIPE.
    let mut input = BufReader::new(stdout);
    let result = (|| {
        let mut items = BTreeSet::new();
        let mut names: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut scanned = 0u64;
        loop {
            let mut header = String::new();
            let read = (&mut input)
                .take(256)
                .read_line(&mut header)
                .map_err(|_| refused())?;
            if read == 0 {
                break;
            }
            if !header.ends_with('\n') {
                return Err(refused());
            }
            let fields = header.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 3 {
                return Err(refused());
            }
            let size: u64 = fields[2].parse().map_err(|_| refused())?;
            if size > MAX_FILE {
                return Err(admission_error(
                    "Copy history object exceeds the 16 MiB per-file limit",
                ));
            }
            scanned = scanned.saturating_add(size).saturating_add(read as u64);
            if scanned > MAX_SCAN {
                return Err(admission_error(
                    "Copy history exceeds the 1 GiB scan-byte limit",
                ));
            }
            if start.elapsed() > scan_limit {
                return Err(admission_error(
                    "Copy history scan exceeded the 120 second time limit",
                ));
            }
            let mut bytes = Zeroizing::new(vec![0; size as usize]);
            input.read_exact(&mut bytes).map_err(|_| refused())?;
            let mut newline = [0u8];
            input.read_exact(&mut newline).map_err(|_| refused())?;
            if newline != *b"\n" {
                return Err(refused());
            }
            if fields[1] == "tree" {
                let oid_width = fields[0].len() / 2;
                let mut remaining = bytes.as_slice();
                while !remaining.is_empty() {
                    let nul = remaining.iter().position(|b| *b == 0).ok_or_else(refused)?;
                    let (mode, name) = std::str::from_utf8(&remaining[..nul])
                        .map_err(|_| refused())?
                        .split_once(' ')
                        .ok_or_else(refused)?;
                    if validate_bytes(name, b"").is_err() {
                        items.insert(VaultItem {
                            path: format!("git-history/{}/{name}", fields[0]),
                            kind: "history path".into(),
                        });
                    }
                    let entry = remaining
                        .get(nul + 1..nul + 1 + oid_width)
                        .ok_or_else(refused)?;
                    if mode != "40000" {
                        names
                            .entry(entry.iter().map(|byte| format!("{byte:02x}")).collect())
                            .or_default()
                            .insert(name.to_owned());
                    }
                    remaining = remaining.get(nul + 1 + oid_width..).ok_or_else(refused)?;
                }
            } else if matches!(fields[1], "blob" | "commit" | "tag") {
                let path = paths
                    .get(fields[0])
                    .cloned()
                    .unwrap_or_else(|| format!("git-history/{}", fields[0]));
                // The same blob is classified under the policy of every name.
                let aliases = names.remove(fields[0]).unwrap_or_default();
                let aliases = aliases
                    .into_iter()
                    .filter(|name| Some(name.as_str()) != path.rsplit('/').next())
                    .map(|name| format!("git-history/{}/{name}", fields[0]))
                    .collect::<Vec<_>>();
                for path in std::iter::once(path).chain(aliases) {
                    if validate_bytes(&path, &bytes).is_err() {
                        items.insert(VaultItem {
                            path,
                            kind: format!("history {}", fields[1]),
                        });
                    }
                }
            } else {
                return Err(refused());
            }
        }
        Ok(items.into_iter().collect())
    })();
    let status = loop {
        let mut owned = child.lock().expect("owned Git child");
        if (result.is_err() || start.elapsed() > scan_limit)
            && !killed_for_refusal.load(Ordering::Relaxed)
            && owned.id() > 1
            && owned.kill().is_ok()
        {
            // The first policy kill owns the reason. A later watchdog must
            // not replace a size/credential refusal with a timeout.
            timed_out.store(result.is_ok(), Ordering::Relaxed);
            killed_for_refusal.store(true, Ordering::Relaxed);
        }
        match owned.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                drop(owned);
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break Err(scan_unavailable()),
        }
    };
    drop(input);
    let _ = done.send(());
    let _ = watchdog.join();
    let _ = writer.join();
    let status = status?;
    scan_git_status(status, killed_for_refusal.load(Ordering::Relaxed))?;
    if timed_out.load(Ordering::Relaxed) {
        return Err(admission_error(
            "Copy history scan exceeded the 120 second time limit",
        ));
    }
    // Preserve the actionable policy reason after our kill; external signals
    // still take precedence in scan_git_status above.
    if !status.success() && result.is_ok() {
        return Err(admission_error(
            "Copy history scan failed or exceeded its 120 second time limit",
        ));
    }
    result
}
