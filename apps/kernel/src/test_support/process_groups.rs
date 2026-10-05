// MP-08/MP-10/MP-11: timeout cleanup cannot signal a system or foreign group.
pub(super) fn owned_group(leader: i32, rows: &[(i32, i32, i32)]) -> bool {
    if leader <= 1 || !rows.iter().any(|row| row.0 == leader && row.2 == leader) {
        return false;
    }
    for &(pid, _, group) in rows.iter().filter(|row| row.2 == leader) {
        if pid <= 1 || group <= 1 {
            return false;
        }
        let mut current = pid;
        let mut seen = std::collections::HashSet::new();
        while current != leader {
            if current <= 1 || !seen.insert(current) {
                return false;
            }
            let Some(row) = rows.iter().find(|row| row.0 == current) else {
                return false;
            };
            current = row.1;
        }
    }
    true
}

pub(super) fn current_group_is_owned(leader: i32) -> bool {
    if leader <= 1 {
        return false;
    }
    let Ok(output) = std::process::Command::new("ps")
        .args(["-eo", "pid=,ppid=,pgid="])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let Ok(text) = String::from_utf8(output.stdout) else {
        return false;
    };
    let mut rows = Vec::new();
    for line in text.lines() {
        let values = line
            .split_whitespace()
            .map(str::parse::<i32>)
            .collect::<Result<Vec<_>, _>>();
        let Ok(values) = values else {
            return false;
        };
        if values.len() != 3 {
            return false;
        }
        rows.push((values[0], values[1], values[2]));
    }
    owned_group(leader, &rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp08_mp10_mp11_group_cleanup_requires_every_member_to_descend_from_owned_child() {
        let owned = [(20, 10, 20), (21, 20, 20), (22, 21, 20)];
        assert!(owned_group(20, &owned));
        for pid in [0, 1, -1, i32::MIN] {
            assert!(!owned_group(pid, &owned));
        }
        assert!(!owned_group(20, &[]));
        assert!(!owned_group(20, &[(20, 10, 20), (30, 1, 20)]));
        assert!(!owned_group(
            20,
            &[(20, 10, 20), (30, 31, 20), (31, 30, 20)]
        ));
        assert!(!owned_group(20, &[(20, 10, 99)]));
    }
}

#[cfg(all(test, unix))]
#[test]
fn mp08_mp10_mp11_live_group_guard_recognizes_only_its_owned_child() {
    use std::os::unix::process::CommandExt;
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .process_group(0)
        .spawn()
        .unwrap();
    let pid = i32::try_from(child.id()).unwrap();
    assert!(pid > 1);
    let owned = current_group_is_owned(pid);
    let refuses_system =
        !current_group_is_owned(0) && !current_group_is_owned(1) && !current_group_is_owned(-1);
    if child.id() > 1 {
        let _ = child.kill();
    }
    child.wait().unwrap();
    assert!(owned && refuses_system);
}
