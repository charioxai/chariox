#!/usr/bin/env python3
"""MP-08/MP-10/MP-11: read only public KA receipt fields; never raw payloads."""
import argparse
import json
from pathlib import Path
import sqlite3

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--database", required=True, type=Path)
parser.add_argument("--after-sequence", type=int, default=0)
parser.add_argument("--checkpoint", action="store_true")
args = parser.parse_args()
db = args.database.resolve(strict=True)
connection = sqlite3.connect(db.as_uri() + "?mode=ro", uri=True)
connection.execute("PRAGMA query_only=ON")
if args.checkpoint:
    print(json.dumps({"mp": ["MP-08", "MP-10", "MP-11"], "after_sequence": connection.execute(
        "SELECT COALESCE(MAX(sequence), 0) FROM durable_state_events").fetchone()[0]}))
else:
    # SQLite projects individual non-secret keys; no whole payload enters Python.
    paths = ["outcome", "reason", "session_id", "interaction_id", "choice_id", "connection_class",
             "turn.entry_id", "turn.session_id", "turn.agent_id", "turn.terminal_id",
             "turn.provider_run_id", "turn.prompt_id", "turn.requester.grant_id",
             "grant.grant_id", "grant.holder_pid", "grant.lifetime_minutes"]
    columns = ", ".join("json_extract(payload_json, '$." + path + "')" for path in paths)
    query = "SELECT sequence, event_id, kind, subject_id, timestamp_ms, " + columns + " FROM durable_state_events WHERE sequence > ? AND kind IN (?, ?, ?, ?, ?) ORDER BY sequence LIMIT 2000"
    kinds = ("kernel_access.grant", "kernel_access.sudo", "kernel_access.sudo_approval",
             "critical_approval.passkey", "critical_approval.passkey_rotation")
    for row in connection.execute(query, (args.after_sequence, *kinds)):
        record = dict(zip(["sequence", "event_id", "kind", "subject_id", "timestamp_ms", *paths], row))
        print(json.dumps({"mp": ["MP-08", "MP-10", "MP-11"], **{k: v for k, v in record.items() if v is not None}}))
connection.close()
