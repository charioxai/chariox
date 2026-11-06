#!/bin/bash
# Waits up to $1 seconds for a new line in interactions.jsonl or agentstate.jsonl (ROOM_DIR), then prints what is new.
cd "${ROOM_DIR:?Set ROOM_DIR}"
a=$(wc -l < interactions.jsonl); b=$(wc -l < agentstate.jsonl)
timeout "${1:-280}" bash -c "until [ \$(wc -l < interactions.jsonl) -gt $a ] || [ \$(wc -l < agentstate.jsonl) -gt $b ]; do sleep 3; done"
tail -n +$((a + 1)) interactions.jsonl | cut -c1-260
tail -n +$((b + 1)) agentstate.jsonl | cut -c1-260
