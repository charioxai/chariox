-- MP-08 / MP-10 / MP-11, A09: FTS only sees sanitized, versioned public text.
PRAGMA secure_delete=ON;
PRAGMA temp_store=MEMORY;
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
CREATE VIRTUAL TABLE IF NOT EXISTS public_history_fts USING fts5(text,content='public_history',content_rowid='sequence',tokenize='unicode61');
INSERT INTO public_history_fts(public_history_fts,rank) VALUES('secure-delete',1);
CREATE TRIGGER IF NOT EXISTS public_history_insert AFTER INSERT ON public_history BEGIN
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
CREATE TRIGGER IF NOT EXISTS public_history_source_update AFTER UPDATE ON history_events BEGIN
 DELETE FROM public_history WHERE event_ref=old.event_id;
 INSERT INTO public_history_revision(session_id,revision) SELECT old.session_id,1 WHERE old.session_id IS NOT NULL
 ON CONFLICT(session_id) DO UPDATE SET revision=revision+1;
END;
