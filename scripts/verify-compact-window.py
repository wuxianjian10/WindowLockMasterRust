"""Check the packaged HWND on a real monitor with a simulated small work area."""
import ctypes as c
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

root = Path(__file__).resolve().parents[1]
report = json.loads((root / 'artifacts/latest-package.json').read_text())
exe = Path(report['directory']) / 'WindowLockMasterRust.exe'
u = c.WinDLL('user32', use_last_error=True)
u.SetProcessDpiAwarenessContext.argtypes = [c.c_void_p]
u.SetProcessDpiAwarenessContext(c.c_void_p(-4))
u.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
u.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
u.GetClassNameW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
u.GetWindowRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
u.MonitorFromPoint.argtypes = [w.POINT, w.DWORD]
u.MonitorFromPoint.restype = w.HMONITOR
u.MonitorFromWindow.argtypes = [w.HWND, w.DWORD]
u.MonitorFromWindow.restype = w.HMONITOR
u.GetDpiForWindow.argtypes = [w.HWND]
u.GetDpiForWindow.restype = w.UINT
u.IsWindowVisible.argtypes = [w.HWND]
u.IsIconic.argtypes = [w.HWND]
u.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
u.SendMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
u.SendMessageW.restype = w.LPARAM


class MonitorInfo(c.Structure):
    _fields_ = [('size', w.DWORD), ('monitor', w.RECT), ('work', w.RECT), ('flags', w.DWORD)]


u.GetMonitorInfoW.argtypes = [w.HMONITOR, c.POINTER(MonitorInfo)]
monitor_callback = c.WINFUNCTYPE(w.BOOL, w.HMONITOR, w.HDC, c.POINTER(w.RECT), w.LPARAM)
u.EnumDisplayMonitors.argtypes = [w.HDC, c.POINTER(w.RECT), monitor_callback, w.LPARAM]
monitors = []


@monitor_callback
def collect_monitor(handle, *_):
    info = MonitorInfo(size=c.sizeof(MonitorInfo))
    if u.GetMonitorInfoW(handle, c.byref(info)):
        monitors.append((handle, info))
    return True


assert u.EnumDisplayMonitors(None, None, collect_monitor, 0)
secondary, info = next((h, i) for h, i in monitors if not i.flags & 1)
work = info.work
assert work.right - work.left >= 800 and work.bottom - work.top >= 520

def wait_for(check, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(.025)
    raise AssertionError(f'timed out at {check.__name__}')


def handles(pid):
    found = {}
    callback = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)

    @callback
    def collect(hwnd, _):
        owner = w.DWORD()
        u.GetWindowThreadProcessId(hwnd, c.byref(owner))
        if owner.value == pid:
            title = c.create_unicode_buffer(256)
            cls = c.create_unicode_buffer(256)
            u.GetWindowTextW(hwnd, title, len(title))
            u.GetClassNameW(hwnd, cls, len(cls))
            if title.value == 'WindowLockMaster Rust ' + report['version']:
                found['main'] = hwnd
            if cls.value == 'WindowLockMasterTray':
                found['tray'] = hwnd
        return True

    u.EnumWindows(collect, 0)
    return found


def check_bounds(hwnd, width, height):
    assert u.MonitorFromWindow(hwnd, 0) == secondary, 'window changed screens'
    rect = w.RECT()
    assert u.GetWindowRect(hwnd, c.byref(rect))
    assert work.left <= rect.left < rect.right <= work.left + width, tuple_rect(rect)
    assert work.top <= rect.top < rect.bottom <= work.top + height, tuple_rect(rect)
    return tuple_rect(rect)


def tuple_rect(rect):
    return rect.left, rect.top, rect.right, rect.bottom


data = Path(tempfile.mkdtemp(prefix='compact-verified-', dir=root / 'artifacts'))
limits = [(800, 520), (640, 440)]
for width, height in limits:
    case = data / f'{width}x{height}'
    env = dict(os.environ, WLM_TEST_WORK=f'{work.left + 48},{work.top + 48},{width},{height}')
    with subprocess.Popen([str(exe), '--readonly', '--data-dir', str(case)], cwd=exe.parent, env=env) as proc:
        tray = None
        try:
            def visible():
                hwnd = handles(proc.pid).get('main')
                return hwnd if hwnd and u.IsWindowVisible(hwnd) and not u.IsIconic(hwnd) else None

            hwnd = wait_for(visible)
            check_bounds(hwnd, width, height)
            tray = wait_for(lambda: handles(proc.pid).get('tray'))
            assert u.SendMessageW(hwnd, 0x007F, 0, 0), 'missing small window icon'
            assert u.SendMessageW(hwnd, 0x007F, 1, 0), 'missing large window icon'
            # WinUI can complete its first layout after the HWND becomes visible.
            # Require containment throughout and a stable frame after layout.
            samples = []
            for _ in range(32):
                samples.append(check_bounds(hwnd, width, height))
                time.sleep(.05)
            settled = samples[-1]
            assert all(frame == settled for frame in samples[-6:]), samples
            assert u.PostMessageW(hwnd, 0x0010, 0, 0)
            wait_for(lambda: not u.IsWindowVisible(hwnd))
            assert u.PostMessageW(tray, 0x0401, 1, 0x0203)
            wait_for(visible)
            assert check_bounds(hwnd, width, height) == settled
            print(f'{width}x{height}: secondary monitor, DPI={u.GetDpiForWindow(hwnd)}, bounds={settled}, icon present, tray restore OK')
        finally:
            if tray:
                u.PostMessageW(tray, 0x0111, 2, 0)
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.terminate()
                proc.wait(timeout=5)
        assert proc.returncode == 0, proc.returncode
        log = (case / 'ui.log').read_text(encoding='utf-8')
        assert 'TRAY_OPEN' in log and 'TRAY_EXIT' in log and 'ERROR' not in log, log
