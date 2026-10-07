-- MP-08 / MP-10 / MP-11, A09: FTS only sees sanitized, versioned public text.
CREATE TABLE IF NOT EXISTS public_history_version(version INTEGER NOT NULL);
INSERT INTO public_history_version SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM public_history_version);
CREATE TABLE IF NOT EXISTS public_history_build(id INTEGER PRIMARY KEY CHECK(id=1),cursor INTEGER NOT NULL,complete INTEGER NOT NULL,epoch INTEGER NOT NULL);
INSERT OR IGNORE INTO public_history_build VALUES(1,0,1,0);
CREATE TABLE IF NOT EXISTS public_history_retention(session_id TEXT PRIMARY KEY,deleted_events INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS public_history_revision(session_id TEXT PRIMARY KEY,revision INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS public_history(
 sequence INTEGER PRIMARY KEY,event_ref TEXT UNIQUE NOT NULL,owner_user_id TEXT NOT NULL,
 session_id TEXT NOT NULL,agent_id TEXT NOT NULL,kind TEXT NOT NULL,text TEXT NOT NULL,document_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS public_history_scope ON public_history(owner_user_id,session_id,agent_id,sequence);
CREATE INDEX IF NOT EXISTS public_history_turn_scope ON public_history(owner_user_id,session_id,agent_id,json_extract(document_json,'$.turn_id'),sequence);
CREATE VIRTUAL TABLE IF NOT EXISTS public_history_fts USING fts5(text,content='public_history',content_rowid='sequence',tokenize='unicode61');
INSERT INTO public_history_fts(public_history_fts,rank) VALUES('secure-delete',1);
-- Refresh trigger definitions on existing databases without discarding sanitized rows.
DROP TRIGGER IF EXISTS public_history_insert;
CREATE TRIGGER public_history_insert AFTER INSERT ON public_history BEGIN
 -- A reservation can commit after a larger sequence. OFFSET cursors for this
 -- room must restart when a late row enters their retained sequence window.
 INSERT INTO public_history_revision(session_id,revision)
 SELECT new.session_id,1 WHERE EXISTS(
  SELECT 1 FROM public_history WHERE session_id=new.session_id AND sequence>new.sequence
 ) ON CONFLICT(session_id) DO UPDATE SET revision=revision+1;
 INSERT INTO public_history_fts(rowid,text) SELECT new.sequence,new.text FROM public_history_build WHERE complete=1 OR new.sequence<=cursor;
END;
CREATE TRIGGER IF NOT EXISTS public_history_delete AFTER DELETE ON public_history BEGIN
 INSERT INTO public_history_fts(public_history_fts,rowid,text) SELECT 'delete',old.sequence,old.text FROM public_history_build WHERE complete=1 OR old.sequence<=cursor;
END;
CREATE TRIGGER IF NOT EXISTS public_history_source_delete AFTER DELETE ON history_events BEGIN
 INSERT INTO public_history_retention(session_id,deleted_events) SELECT old.session_id,1 WHERE old.session_id IS NOT NULL
 ON CONFLICT(session_id) DO UPDATE SET deleted_events=deleted_events+1;
 DELETE FROM public_history WHERE event_ref=old.event_id;
 INSERT INTO public_history_revision(session_id,revision) SELECT old.session_id,1 WHERE old.session_id IS NOT NULL
 ON CONFLICT(session_id) DO UPDATE SET revision=revision+1;
END;
-- Commit order and leased projection keys are bookkeeping, not public authority.
DROP TRIGGER IF EXISTS public_history_source_update;
CREATE TRIGGER public_history_source_update AFTER UPDATE ON history_events
WHEN old.event_id IS NOT new.event_id
 OR old.sequence IS NOT new.sequence
 OR old.timestamp_ms IS NOT new.timestamp_ms
 OR old.kind IS NOT new.kind
 OR old.session_id IS NOT new.session_id
 OR old.agent_id IS NOT new.agent_id
 OR old.provider IS NOT new.provider
 OR old.model IS NOT new.model
 OR old.turn_id IS NOT new.turn_id
 OR old.prompt_id IS NOT new.prompt_id
 OR old.provider_run_id IS NOT new.provider_run_id
 OR old.workflow_id IS NOT new.workflow_id
 OR old.workflow_run_id IS NOT new.workflow_run_id
 OR old.workflow_node_id IS NOT new.workflow_node_id
 OR old.machine_id IS NOT new.machine_id
 OR old.repo_root IS NOT new.repo_root
 OR old.worktree_path IS NOT new.worktree_path
 OR old.content IS NOT new.content
 OR old.content_ref IS NOT new.content_ref
 OR old.metadata_text IS NOT new.metadata_text
 OR old.merge_key IS NOT new.merge_key
 OR old.event_json IS NOT new.event_json
BEGIN
 DELETE FROM public_history WHERE event_ref=old.event_id;
 INSERT INTO public_history_revision(session_id,revision) SELECT old.session_id,1 WHERE old.session_id IS NOT NULL
 ON CONFLICT(session_id) DO UPDATE SET revision=revision+1;
END;
