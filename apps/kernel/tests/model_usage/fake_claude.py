#!/usr/bin/python3
"""Emit Claude stream-json results with cumulative per-model token counts."""
import json
import os
from pathlib import Path
import sys


def emit(**frame):
    print(json.dumps(frame), flush=True)


args = sys.argv[1:]
if args == ["--version"]:
    print("Claude Code 2.1.207")
    sys.exit()
if args[:2] == ["auth", "status"]:
    emit(loggedIn=True, authMethod="claude.ai", subscriptionType="pro")
    sys.exit()
if any("CHARIOX_AUTH_OK" in arg for arg in args):
    emit(type="result", subtype="success", is_error=False, result="CHARIOX_AUTH_OK")
    sys.exit()
if "stream-json" not in args or "--no-session-persistence" in args:
    sys.exit()

is_resume = "--resume" in args
session_arg = "--resume" if is_resume else "--session-id"
session = args[args.index(session_arg) + 1]
record = Path(os.environ["CHARIOX_MODEL_USAGE_FIXTURE"])
haiku = "claude-haiku-5-5"
opus = "claude-opus-4-6"
sonnet = "claude-sonnet-4-6"
cumulative = [{haiku: 100, opus: 40}, {haiku: 250, opus: 40, sonnet: 20},
              {haiku: 310, sonnet: 20}, {haiku: 15}]
for line in sys.stdin:
    frame = json.loads(line)
    if frame["type"] != "user":
        continue
    turn = int(frame["message"]["content"][0]["text"].split("usage turn ")[-1].split()[0])
    if turn == 4:
        session = "11111111-2222-4333-8444-555555555555"
    (record / f"turn-{turn}").write_text(f"{session} {is_resume}")
    emit(type="system", subtype="init", session_id=session, model=haiku)
    emit(type="assistant", session_id=session,
         message={"id": f"usage-{turn}", "role": "assistant", "model": haiku,
                  "content": [{"type": "text", "text": "Done"}]})
    emit(type="result", subtype="success", is_error=False, session_id=session, result="Done",
         user_message_uuids=[frame["uuid"]],
         modelUsage={model: {"inputTokens": count - count // 10, "outputTokens": count // 5,
                             "cacheReadInputTokens": count // 10, "cacheCreationInputTokens": 0}
                     for model, count in cumulative[turn - 1].items()})
