"""Calls the kernel's chariox.vault.generate tool `count` times over the MCP
server this provider process was given (its --mcp-config), so the Room holds
protected values like a working Room does. Drill use only."""
import json, sys, urllib.request

config_path, count = sys.argv[1], int(sys.argv[2])
servers = json.load(open(config_path))["mcpServers"]
server = servers.get("chariox") or next(iter(servers.values()))
url, headers = server["url"], dict(server.get("headers", {}))
headers.update({"Content-Type": "application/json", "Accept": "application/json, text/event-stream"})
session = None

def call(method, params, ident=None):
    global session
    body = {"jsonrpc": "2.0", "method": method, "params": params}
    if ident is not None:
        body["id"] = ident
    request_headers = dict(headers)
    if session:
        request_headers["Mcp-Session-Id"] = session
    request = urllib.request.Request(url, json.dumps(body).encode(), request_headers)
    with urllib.request.urlopen(request, timeout=60) as response:
        session = response.headers.get("Mcp-Session-Id") or session
        text = response.read().decode()
    for line in text.splitlines():
        if line.startswith("data:"):
            text = line[5:]
    return json.loads(text) if text.strip() else None

call("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "drill-seed", "version": "1"}}, 1)
call("notifications/initialized", {})
tools = call("tools/list", {}, 2)["result"]["tools"]
name = next(tool["name"] for tool in tools if "vault" in tool["name"] and "generate" in tool["name"])
for index in range(count):
    result = call("tools/call", {"name": name, "arguments": {"request_id": f"drill-seed-{index}", "origin": f"https://drill-seed-{index}.example.com"}}, 10 + index)
    print(json.dumps(result)[:160], file=sys.stderr)
