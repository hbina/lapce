#!/usr/bin/env python3
"""Check declared crate boundaries without fetching or building dependencies."""
import argparse
import json
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--resolved", action="store_true", help="Also inspect feature-resolved process-only dependencies (requires cached dependencies)")
args = parser.parse_args()

root = Path(__file__).resolve().parents[1]
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--offline", "--no-deps", "--format-version", "1"],
    cwd=root, text=True,
))
packages = {package["name"]: package for package in metadata["packages"]}

root_binaries = {target["name"] for target in packages["lapce"]["targets"]
                 if "bin" in target["kind"]}
assert root_binaries == {"lapce-floem", "lapce-iced", "lapce-proxy"}, \
    f"Unexpected root executables: {sorted(root_binaries)}"
for name in ("lapce-app", "lapce-iced", "lapce-proxy"):
    assert not any("bin" in target["kind"] for target in packages[name]["targets"]), \
        f"{name} must expose its executable through the root lapce package"

def dependencies(name):
    return [dependency for dependency in packages[name]["dependencies"]
            if dependency["kind"] != "dev"]

def reachable(name, seen=None):
    seen = set() if seen is None else seen
    for dependency in dependencies(name):
        target = dependency["name"]
        if target in seen:
            continue
        seen.add(target)
        if target in packages:
            reachable(target, seen)
    return seen

for name in ("lapce-engine", "lapce-services", "lapce-core", "lapce-rpc"):
    forbidden = reachable(name) & {"lapce-app", "lapce-iced", "lapce-client", "lapce-proxy", "floem", "iced"}
    assert not forbidden, f"{name} crosses shared boundary: {sorted(forbidden)}"
assert "lapce-proxy" not in {d["name"] for d in dependencies("lapce-app")}, \
    "Desktop must connect through lapce-client"
client = packages["lapce-client"]
assert not client["features"].get("default"), "Backend embedding must be opt-in"
proxy = next(d for d in dependencies("lapce-client") if d["name"] == "lapce-proxy")
assert proxy["optional"], "Client backend dependency must remain optional"
assert not (reachable("lapce-client") & {"lapce-app", "lapce-iced", "lapce-engine", "floem", "iced"}), \
    "Connection library must remain independent of frontends and engine"
assert not (reachable("lapce-iced") & {"lapce-app", "floem"}), \
    "Iced frontend must remain independent of the Floem desktop"
assert "lapce-proxy" not in {d["name"] for d in dependencies("lapce-iced")}, \
    "Iced frontend must connect through lapce-client"
if args.resolved:
    for frontend in ("lapce-client", "lapce-app", "lapce-iced"):
        tree = subprocess.check_output(
            ["cargo", "tree", "--offline", "-p", frontend, "--no-default-features",
             "--edges", "normal,build", "--prefix", "none", "--format", "{p}"],
            cwd=root, text=True,
        )
        names = {line.split()[0] for line in tree.splitlines() if line.strip()}
        forbidden = names & {"lapce-proxy", "wasmtime", "wasi-common"}
        assert not forbidden, f"{frontend} process-only build embeds backend: {sorted(forbidden)}"
        if frontend == "lapce-iced":
            assert not (names & {"lapce-app", "floem"}), "Iced build pulls in Floem desktop"
        if frontend == "lapce-client":
            assert not (names & {"lapce-app", "lapce-engine", "floem"}), "Process client pulls in a frontend"
print("Architecture boundaries passed")
