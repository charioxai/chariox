//! MP-08 / MP-11: deterministic official-CLI seam for isolated source tests only.
use std::{ffi::OsString, os::unix::fs::PermissionsExt};

pub(crate) struct TestClaudeExecutable {
    _root: super::TestWorktree,
    previous: Option<OsString>,
}

impl TestClaudeExecutable {
    pub(crate) fn new() -> Self {
        assert!(super::environment_test_isolated());
        let root = super::TestWorktree::new("claude-executable");
        let executable = root.path().join("claude");
        std::fs::write(&executable, r#"#!/bin/sh
if [ "$1" = --version ]; then echo 2.2.0; exit 0; fi
stream=0
for argument in "$@"; do
  [ "$argument" = --input-format ] && stream=1
done
if [ "$1" = -p ] && [ "$stream" = 0 ]; then
  printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"num_turns":1,"usage":{"output_tokens":7},"result":"CHARIOX_AUTH_OK"}'
  exit 0
fi
while IFS= read -r line; do :; done
"#).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let previous = std::env::var_os("CHARIOX_CLAUDE_BIN");
        std::env::set_var("CHARIOX_CLAUDE_BIN", &executable);
        Self {
            _root: root,
            previous,
        }
    }
}

impl Drop for TestClaudeExecutable {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(previous) => std::env::set_var("CHARIOX_CLAUDE_BIN", previous),
            None => std::env::remove_var("CHARIOX_CLAUDE_BIN"),
        }
    }
}
