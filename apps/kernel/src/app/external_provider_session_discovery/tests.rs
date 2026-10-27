use super::*;
use crate::history::SessionHistoryEntryKind;
use std::fs::OpenOptions;
use std::io::{self, Write};

mod claude;
mod codex;
mod common;
mod opencode;

pub(super) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(super) fn temp_dir(name: &str) -> TempDir {
    let path = env::temp_dir().join(format!("chariox-{name}-{}", unix_epoch_ms()));
    match fs::create_dir_all(&path) {
        Ok(()) => TempDir { path },
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let fallback =
                env::temp_dir().join(format!("chariox-{name}-{}-fallback", unix_epoch_ms()));
            fs::create_dir_all(&fallback).unwrap();
            TempDir { path: fallback }
        }
        Err(error) => panic!("failed to create temp dir: {error}"),
    }
}

pub(super) fn seed_opencode_sqlite(path: &Path) {
    let connection = Connection::open(path).expect("sqlite fixture should open");
    connection
            .execute_batch(
                r#"
                create table session (
                    id text primary key,
                    project_id text not null,
                    parent_id text,
                    slug text not null,
                    directory text not null,
                    title text not null,
                    version text not null,
                    share_url text,
                    summary_additions integer,
                    summary_deletions integer,
                    summary_files integer,
                    summary_diffs text,
                    revert text,
                    permission text,
                    time_created integer not null,
                    time_updated integer not null,
                    time_compacting integer,
                    time_archived integer,
                    workspace_id text
                );
                create table message (
                    id text primary key,
                    session_id text not null,
                    time_created integer not null,
                    time_updated integer not null,
                    data text not null
                );
                create table part (
                    id text primary key,
                    message_id text not null,
                    session_id text not null,
                    time_created integer not null,
                    time_updated integer not null,
                    data text not null
                );
                insert into session (
                    id, project_id, slug, directory, title, version,
                    time_created, time_updated
                ) values (
                    'ses_sqlite_1', 'project_1', 'sqlite-imports',
                    '/repo/sqlite', 'SQLite OpenCode import', '0.0.0',
                    1782113000000, 1782113050000
                );
                insert into message (
                    id, session_id, time_created, time_updated, data
                ) values (
                    'msg_user', 'ses_sqlite_1', 1782113001000, 1782113001000,
                    '{"role":"user"}'
                );
                insert into part (
                    id, message_id, session_id, time_created, time_updated, data
                ) values (
                    'prt_user_text', 'msg_user', 'ses_sqlite_1',
                    1782113001001, 1782113001001,
                    '{"type":"text","text":"Investigate SQLite-backed OpenCode imports."}'
                );
                insert into message (
                    id, session_id, time_created, time_updated, data
                ) values (
                    'msg_assistant', 'ses_sqlite_1', 1782113002000, 1782113003000,
                    '{"role":"assistant","modelID":"kimi-k2.6","tokens":{"input":10,"output":5},"time":{"completed":1782113003000},"finish":"stop"}'
                );
                insert into part (
                    id, message_id, session_id, time_created, time_updated, data
                ) values (
                    'prt_reasoning', 'msg_assistant', 'ses_sqlite_1',
                    1782113002001, 1782113002001,
                    '{"type":"reasoning","text":"Internal reasoning"}'
                );
                insert into part (
                    id, message_id, session_id, time_created, time_updated, data
                ) values (
                    'prt_tool', 'msg_assistant', 'ses_sqlite_1',
                    1782113002002, 1782113002002,
                    '{"type":"tool","tool":"bash","state":{"status":"completed","input":{"command":"printf TOOL_STEP_01"},"output":"created"}}'
                );
                insert into part (
                    id, message_id, session_id, time_created, time_updated, data
                ) values (
                    'prt_assistant_text', 'msg_assistant', 'ses_sqlite_1',
                    1782113003000, 1782113003000,
                    '{"type":"text","text":"Use the session, message, and part tables."}'
                );
                "#,
            )
            .expect("sqlite fixture should seed");
}
