#!/usr/bin/env python3
"""Calls one kalangod tool over its Unix socket, as an agent would (tests). usage: mcp-chamar.py <socket> <tool> '<json args>'
Prints the result as JSON; the exit status is 0 when the tool did not fail."""
import json, socket, sys

sock, tool, args = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
s = socket.socket(socket.AF_UNIX)
s.connect(sock)
f = s.makefile("rw")
f.write(json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}}) + "\n")
f.flush()
r = json.loads(f.readline()).get("result", {})
print(json.dumps(r, ensure_ascii=False))
sys.exit(1 if r.get("isError") else 0)
