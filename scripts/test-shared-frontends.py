#!/usr/bin/env python3
"""Smoke-test two independent terminal frontend processes sharing one host."""
from pathlib import Path
import selectors
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
binaries = root / "target" / "ci"
with tempfile.TemporaryDirectory(prefix="lapce-shared-") as temporary:
    workspace = Path(temporary)
    source = workspace / "file.rs"
    source.write_text("shared🌍\n")
    socket = workspace / "backend.sock"
    processes = []
    try:
        host = subprocess.Popen([str(binaries / "examples" / "shared_host"), str(binaries / "lapce-proxy"), str(workspace), str(socket)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        processes.append(host)
        ready = selectors.DefaultSelector()
        ready.register(host.stdout, selectors.EVENT_READ)
        assert ready.select(timeout=10), "host did not announce readiness"
        assert "listening" in host.stdout.readline(), "host failed to start"
        ready.close()
        command = [str(binaries / "examples" / "terminal"), "--socket", str(socket), str(source)]
        first = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        processes.append(first)
        ready = selectors.DefaultSelector()
        ready.register(first.stdout, selectors.EVENT_READ)
        assert ready.select(timeout=10), "first frontend did not start"
        assert "Commands:" in first.stdout.readline()
        ready.close()
        second = subprocess.run(command, input="show\nquit\n", capture_output=True, text=True, timeout=10)
        assert second.returncode == 0, second.stderr
        assert "shared🌍" in second.stdout, second.stdout
        output, errors = first.communicate("show\nquit\n", timeout=10)
        assert first.returncode == 0, errors
        assert "shared🌍" in output, output
        output, errors = host.communicate("quit\n", timeout=10)
        assert host.returncode == 0, errors
        assert not socket.exists(), "host did not remove its socket"
        assert source.read_text() == "shared🌍\n"
        print("Two independent frontend processes shared one backend successfully")
    finally:
        for process in reversed(processes):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
