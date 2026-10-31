"""Deterministic LSP fixture: gate semantic replies to exercise reload races."""
import json
from pathlib import Path
import sys
import time

root = Path(sys.argv[1])
semantic_requests = 0


def respond(request_id, result=None, error=None):
    message = {"jsonrpc": "2.0", "id": request_id}
    message["error" if error else "result"] = error or result
    body = json.dumps(message).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()


while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line in (b"\r\n", b"\n"):
            break
        key, value = line.decode().split(":", 1)
        headers[key.lower()] = value.strip()
    request = json.loads(sys.stdin.buffer.read(int(headers["content-length"])))
    method = request.get("method")
    if method == "initialize":
        respond(request["id"], {"capabilities": {
            "textDocumentSync": 1,
            "semanticTokensProvider": {
                "legend": {"tokenTypes": ["function"], "tokenModifiers": []},
                "full": True,
            },
        }})
    elif method == "textDocument/semanticTokens/full":
        semantic_requests += 1
        number = semantic_requests
        (root / f"semantic-{number}.pending").touch()
        if number <= 2:
            deadline = time.monotonic() + 15
            while not (root / f"semantic-{number}.release").exists():
                if time.monotonic() > deadline:
                    sys.exit("semantic test gate timed out")
                time.sleep(0.005)
            respond(request["id"], {"data": [0, 0, 2, 0, 0]})
        else:
            respond(request["id"], error={"code": -32603, "message": "fixture semantic failure"})
        (root / f"semantic-{number}.done").touch()
    elif method == "shutdown":
        respond(request["id"])
    elif method == "exit":
        break
    elif "id" in request:
        respond(request["id"])
