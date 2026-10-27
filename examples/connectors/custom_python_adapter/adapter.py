#!/usr/bin/env python3
import json
import sys


def respond(request_id, ok=True, result=None, error=None):
    print(json.dumps({"id": request_id, "ok": ok, "result": result, "error": error}), flush=True)


for line in sys.stdin:
    if not line.strip():
        continue
    request = json.loads(line)
    request_id = request.get("id", "")
    request_type = request.get("type")
    if request_type == "shutdown":
        break
    if request_type == "validate":
        respond(request_id, result={"validated": True})
        continue
    if request_type == "prepare":
        respond(request_id, result={
            "credential_targets": [],
            "prepared_config": request.get("config") or {},
        })
        continue
    if request_type != "call":
        respond(request_id, ok=False, error=f"unsupported request type {request_type}")
        continue
    credential = request.get("credential") or {}
    respond(request_id, result={
        "connector": request.get("connector"),
        "operation": request.get("operation"),
        "arguments": request.get("arguments") or {},
        "config": request.get("config") or {},
        "credential_id": credential.get("id"),
        "has_secret": bool(credential.get("secret")),
    })
