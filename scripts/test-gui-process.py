#!/usr/bin/env python3
"""Manual GUI smoke test for a process-only Debug build on Linux X11."""
import argparse, ctypes, json, os, re, struct, subprocess, tempfile, time
from pathlib import Path
from PIL import Image
parser = argparse.ArgumentParser(description="Linux X11 GUI process-mode smoke test; requires Pillow, xwininfo, xprop, xwd and libXtst. Captures need visual review.")
repository = Path(__file__).resolve().parents[1]
parser.add_argument('--lapce', type=Path, default=repository/'target/ci/lapce-floem')
parser.add_argument('--proxy', type=Path, default=repository/'target/ci/lapce-proxy')
parser.add_argument('--output-directory', type=Path)
parser.add_argument('--lsp', type=Path, help='Optional rust-analyzer executable for live GUI definition navigation')
args = parser.parse_args()
root = (args.output_directory or Path(tempfile.mkdtemp(prefix='lapce-gui-validation-'))).resolve()
(root/'config/lapce-debug').mkdir(parents=True, exist_ok=True)
(root/'workspace').mkdir(parents=True, exist_ok=True)
(root/'config/lapce-debug/settings.toml').write_text('[core]\nbackend-executable = '+json.dumps(str(args.proxy.resolve()))+'\n')
fixture = 'pub fn answer() -> u32 { 42 }\npub fn use_answer() -> u32 { answer() }\n// GUI process validation 🌍\n'
(root/'workspace/example.rs').write_text(fixture)
if args.lsp:
    with (root/'config/lapce-debug/settings.toml').open('a') as settings:
        settings.write('\n[lapce-native-lsp]\nservers = [{ language = "rust", executable = '+json.dumps(str(args.lsp.absolute()))+' }]\n')
    (root/'workspace/Cargo.toml').write_text('[package]\nname = "gui_navigation_fixture"\nversion = "0.1.0"\nedition = "2021"\n[lib]\npath = "example.rs"\n')
env = os.environ.copy()
env.pop('WAYLAND_DISPLAY', None)
env.update(XDG_CONFIG_HOME=str(root/'config'), XDG_DATA_HOME=str(root/'data'), XDG_CACHE_HOME=str(root/'cache'), WINIT_UNIX_BACKEND='x11')
with (root/'gui-output.log').open('w') as log:
    app = subprocess.Popen([str(args.lapce.resolve()), str(root/'workspace'), str(root/'workspace/example.rs')], env=env, stdout=log, stderr=log)
    try:
        deadline = time.monotonic() + 20
        window = None
        while time.monotonic() < deadline:
            tree = subprocess.run(['xwininfo', '-root', '-tree'], capture_output=True, text=True, env=env).stdout
            for line in tree.splitlines():
                if 'Lapce' in line or 'lapce' in line:
                    match = re.search(r'(0x[0-9a-f]+)', line)
                    if match:
                        pid = subprocess.run(['xprop', '-id', match[1], '_NET_WM_PID'], capture_output=True, text=True, env=env).stdout
                        if str(app.pid) in pid:
                            window = match[1]
                            break
            if window or app.poll() is not None: break
            time.sleep(0.2)
        if not window:
            raise RuntimeError(f'No test window found; process status {app.poll()}')
        time.sleep(2)
        info = subprocess.run(['xwininfo', '-id', window], capture_output=True, text=True, env=env).stdout
        values = [int(re.search(pattern, info)[1]) for pattern in [r'Absolute upper-left X:\s+(-?\d+)',r'Absolute upper-left Y:\s+(-?\d+)',r'Width:\s+(\d+)',r'Height:\s+(\d+)']]
        x,y,w,h = values
        children = subprocess.run(['ps', '--ppid', str(app.pid), '-o', 'pid,comm', '--no-headers'], capture_output=True, text=True).stdout
        print(json.dumps({'window':window, 'running':app.poll() is None, 'children':children.strip(), 'artifact_directory':str(root)}))
        assert 'lapce-proxy' in children, 'No separate backend child found'
        def capture(name):
            assert app.poll() is None, 'Test desktop exited'
            owner = subprocess.run(['xprop', '-id', window, '_NET_WM_PID'], capture_output=True, text=True, env=env).stdout
            assert re.search(r'=\s*'+str(app.pid)+r'\s*$', owner), 'Test window ownership changed'
            subprocess.run(['xwd', '-silent', '-id', window, '-out', str(root/'window.xwd')], env=env, check=True)
            data = (root/'window.xwd').read_bytes()
            header = struct.unpack('>25I', data[:100])
            offset = header[0] + header[19]*12
            assert header[11] == 32 and header[14:17] == (0xff0000, 0xff00, 0xff), header
            Image.frombytes('RGB', (header[4], header[5]), data[offset:], 'raw', 'BGRX' if header[7] == 0 else 'XRGB', header[12], 1).save(root/name)
        capture('initial.png')
        xlib = ctypes.CDLL('libX11.so.6')
        xtest = ctypes.CDLL('libXtst.so.6')
        xlib.XOpenDisplay.argtypes = [ctypes.c_char_p]
        xlib.XOpenDisplay.restype = ctypes.c_void_p
        display = xlib.XOpenDisplay(env['DISPLAY'].encode())
        assert display
        xlib.XStringToKeysym.argtypes = [ctypes.c_char_p]
        xlib.XStringToKeysym.restype = ctypes.c_ulong
        xlib.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        xlib.XKeysymToKeycode.restype = ctypes.c_uint
        xlib.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
        xlib.XRaiseWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        xlib.XFlush.argtypes = [ctypes.c_void_p]
        xlib.XGetInputFocus.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_int)]
        xtest.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        xtest.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        xtest.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
        def key(name, pressed):
            focus, revert = ctypes.c_ulong(), ctypes.c_int()
            xlib.XGetInputFocus(display, ctypes.byref(focus), ctypes.byref(revert))
            assert focus.value == int(window,16), 'Keyboard focus left the test window; stop without sending input'
            symbol = xlib.XStringToKeysym(name.encode())
            code = xlib.XKeysymToKeycode(display, symbol)
            assert code, name
            xtest.XTestFakeKeyEvent(display, code, int(pressed), 0)
            xlib.XFlush(display)
        def tap(name):
            key(name, True); key(name, False)
        def chord(*names):
            for name in names: key(name, True)
            for name in reversed(names): key(name, False)
        def proxy_pid():
            children = subprocess.run(['ps', '--ppid', str(app.pid), '-o', 'pid,comm', '--no-headers'], capture_output=True, text=True).stdout
            return next((int(line.split()[0]) for line in children.splitlines() if line.split()[1] == 'lapce-proxy'), None)
        xlib.XRaiseWindow(display, int(window,16))
        xlib.XSetInputFocus(display, int(window,16), 2, 0)
        xtest.XTestFakeMotionEvent(display, -1, x+450, y+100, 0)
        xtest.XTestFakeButtonEvent(display, 1, 1, 0)
        xtest.XTestFakeButtonEvent(display, 1, 0, 0)
        xlib.XFlush(display)
        time.sleep(.2)
        source = root/'workspace/example.rs'
        original = source.read_text()
        for name in ['k','BackSpace','Delete']: tap(name)
        chord('Control_L','s')
        time.sleep(.5)
        assert source.read_text() == original, 'Typing/save changed disk'
        capture('readonly.png')
        if args.lsp:
            time.sleep(5)
            lsp_children = subprocess.run(['ps', '--ppid', str(proxy_pid()), '-o', 'comm', '--no-headers'], capture_output=True, text=True).stdout
            assert 'rust-analyzer' in lsp_children, 'No native Rust language server child found'
            chord('Control_L', 'Home')
            tap('Down'); tap('End')
            for _ in range(7): tap('Left')
            time.sleep(.2)
            capture('definition-reference.png')
            tap('F12')
            time.sleep(2)
            capture('definition-result.png')
            assert source.read_text() == original
            print(json.dumps({'native_lsp':True, 'definition_captures':['definition-reference.png','definition-result.png']}))
        corrected = original.replace('{ 42 }', '{ 43 }').replace('GUI process validation 🌍', 'external reload verified')
        source.write_text(corrected)
        time.sleep(1.5)
        capture('reload.png')
        old_pid = proxy_pid()
        tap('F1')
        time.sleep(.5)
        for character in 'restart backend':
            tap('space' if character == ' ' else character)
            time.sleep(.01)
        time.sleep(.5)
        capture('restart-command.png')
        tap('Return')
        deadline = time.monotonic()+10
        new_pid = old_pid
        while time.monotonic()<deadline:
            new_pid = proxy_pid()
            if new_pid and new_pid != old_pid: break
            time.sleep(.2)
        assert new_pid and new_pid != old_pid, 'Backend restart did not replace child'
        time.sleep(1.5)
        capture('restart.png')
        if args.lsp:
            time.sleep(5)
            chord('Control_L', 'Home')
            tap('Down'); tap('End')
            for _ in range(7): tap('Left')
            tap('F12')
            time.sleep(2)
            capture('definition-after-restart.png')
        assert source.read_text() == corrected
        print(json.dumps({'readonly_disk':True,'old_proxy':old_pid,'new_proxy':new_pid,'captures':['readonly.png','reload.png','restart-command.png','restart.png']}))
    finally:
        if app.poll() is None:
            app.terminate()
            try: app.wait(timeout=10)
            except subprocess.TimeoutExpired: app.kill(); app.wait()
