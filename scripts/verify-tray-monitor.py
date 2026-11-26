"""Exercise the native tray submenu in a read-only manager test process."""
import ctypes as c
from ctypes import wintypes as w
from pathlib import Path
import sys
import time

pid = int(sys.argv[1])
log = Path(sys.argv[2])
u = c.WinDLL('user32', use_last_error=True)
u.SetProcessDpiAwarenessContext.argtypes = [c.c_void_p]
u.SetProcessDpiAwarenessContext(c.c_void_p(-4))

def bind(name, types, result):
    fn = getattr(u, name)
    fn.argtypes, fn.restype = types, result
    return fn

CB = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
enum_windows = bind('EnumWindows', [CB, w.LPARAM], w.BOOL)
get_pid = bind('GetWindowThreadProcessId', [w.HWND, c.POINTER(w.DWORD)], w.DWORD)
get_class = bind('GetClassNameW', [w.HWND, w.LPWSTR, c.c_int], c.c_int)
get_title = bind('GetWindowTextW', [w.HWND, w.LPWSTR, c.c_int], c.c_int)
visible = bind('IsWindowVisible', [w.HWND], w.BOOL)
iconic = bind('IsIconic', [w.HWND], w.BOOL)
zoomed = bind('IsZoomed', [w.HWND], w.BOOL)
show_async = bind('ShowWindowAsync', [w.HWND, c.c_int], w.BOOL)
post = bind('PostMessageW', [w.HWND, w.UINT, w.WPARAM, w.LPARAM], w.BOOL)
get_monitor = bind('MonitorFromWindow', [w.HWND, w.DWORD], w.HMONITOR)
get_rect = bind('GetWindowRect', [w.HWND, c.POINTER(w.RECT)], w.BOOL)
count = bind('GetMenuItemCount', [w.HMENU], c.c_int)
get_submenu = bind('GetSubMenu', [w.HMENU, c.c_int], w.HMENU)
get_string = bind('GetMenuStringW', [w.HMENU, w.UINT, w.LPWSTR, c.c_int, w.UINT], c.c_int)
get_state = bind('GetMenuState', [w.HMENU, w.UINT, w.UINT], w.UINT)
get_item_rect = bind('GetMenuItemRect', [w.HWND, w.HMENU, w.UINT, c.POINTER(w.RECT)], w.BOOL)
get_cursor = bind('GetCursorPos', [c.POINTER(w.POINT)], w.BOOL)
set_cursor = bind('SetCursorPos', [c.c_int, c.c_int], w.BOOL)
window_at = bind('WindowFromPoint', [w.POINT], w.HWND)
mouse_event = bind('mouse_event', [w.DWORD, w.DWORD, w.DWORD, w.DWORD, c.c_size_t], None)
send_timeout = bind('SendMessageTimeoutW', [w.HWND, w.UINT, w.WPARAM, w.LPARAM, w.UINT, w.UINT, c.POINTER(c.c_size_t)], w.LPARAM)

class MonitorInfo(c.Structure):
    _fields_ = [('size', w.DWORD), ('monitor', w.RECT), ('work', w.RECT), ('flags', w.DWORD), ('device', w.WCHAR * 32)]

get_info = bind('GetMonitorInfoW', [w.HMONITOR, c.POINTER(MonitorInfo)], w.BOOL)

def wait_for(check, description):
    end = time.monotonic() + 6
    while time.monotonic() < end:
        if value := check():
            return value
        time.sleep(.025)
    raise AssertionError(f'timed out: {description}')

def windows():
    items = []
    @CB
    def collect(hwnd, _):
        owner = w.DWORD()
        get_pid(hwnd, c.byref(owner))
        if owner.value == pid:
            cls = c.create_unicode_buffer(256)
            title = c.create_unicode_buffer(256)
            get_class(hwnd, cls, len(cls))
            get_title(hwnd, title, len(title))
            items.append((hwnd, cls.value, title.value))
        return True
    enum_windows(collect, 0)
    return items

def class_window(cls):
    return next((hwnd for hwnd, name, _ in windows() if name == cls), None)

def menu_windows():
    found = {}
    for hwnd, cls, _ in windows():
        if cls == '#32768' and visible(hwnd):
            result = c.c_size_t()
            if send_timeout(hwnd, 0x01E1, 0, 0, 2, 250, c.byref(result)) and result.value:
                found[result.value] = hwnd
    return found

def label(menu, index):
    text = c.create_unicode_buffer(256)
    get_string(menu, index, text, len(text), 0x400)
    return text.value

def select_item(hwnd, menu, index, click):
    rect = w.RECT()
    assert get_item_rect(None, menu, index, c.byref(rect))
    point = w.POINT((rect.left + rect.right) // 2, (rect.top + rect.bottom) // 2)
    assert window_at(point) == hwnd, 'test popup item is obscured'
    assert set_cursor(point.x, point.y)
    if click:
        time.sleep(.12)
        cursor = w.POINT()
        assert get_cursor(c.byref(cursor)) and (cursor.x, cursor.y) == (point.x, point.y)
        assert window_at(point) == hwnd
        mouse_event(2, 0, 0, 0, 0)
        mouse_event(4, 0, 0, 0, 0)

def show_menu(tray):
    work = monitor(main).work
    assert set_cursor(work.left + 200, work.top + 300)
    time.sleep(.12)
    assert post(tray, 0x401, 1, 0x205)
    open_menu = wait_for(menu_windows, 'native tray menu')
    for menu, hwnd in open_menu.items():
        for i in range(count(menu)):
            if label(menu, i) == '移动本程序窗口到显示器':
                submenu = get_submenu(menu, i)
                assert submenu and count(submenu) >= 2
                select_item(hwnd, menu, i, click=False)
                sub_hwnd = wait_for(lambda: menu_windows().get(submenu), 'display submenu')
                return submenu, sub_hwnd
    raise AssertionError('display submenu unavailable')

def monitor(hwnd):
    handle = get_monitor(hwnd, 2)
    info = MonitorInfo(size=c.sizeof(MonitorInfo))
    assert get_info(handle, c.byref(info))
    return info

tray = wait_for(lambda: class_window('WindowLockMasterTray'), 'owned tray HWND')
main = wait_for(lambda: next((hwnd for hwnd, _, title in windows() if title.startswith('WindowLockMaster Rust ')), None), 'owned main HWND')

def move(use_current=False):
    current = monitor(main).device
    submenu, sub_hwnd = show_menu(tray)
    rows = [label(submenu, i) for i in range(count(submenu))]
    selected = [i for i in range(len(rows)) if get_state(submenu, i, 0x400) & 8]
    assert len(selected) == 1 and current.split('\\')[-1] in rows[selected[0]] and '当前' in rows[selected[0]], rows
    index = selected[0] if use_current else next(i for i in range(len(rows)) if i not in selected)
    target = rows[index].split()[0]
    select_item(sub_hwnd, submenu, index, click=True)
    wait_for(lambda: visible(main) and not iconic(main) and monitor(main).device.split('\\')[-1] == target, f'move to {target}')
    wait_for(lambda: not menu_windows(), 'menu dismissal')
    return target, rows

original_cursor = w.POINT()
assert get_cursor(c.byref(original_cursor))
try:
    assert post(main, 0x10, 0, 0)
    wait_for(lambda: not visible(main), 'hide to tray')
    target, rows = move()
    print('Hidden window opened on', target, rows, flush=True)
    time.sleep(.5)
    initial = w.RECT()
    assert get_rect(main, c.byref(initial))
    work = monitor(main).work
    assert work.left <= initial.left < initial.right <= work.right
    assert work.top <= initial.top < initial.bottom <= work.bottom
    move(use_current=True)
    same = w.RECT()
    assert get_rect(main, c.byref(same))
    assert (initial.left, initial.top, initial.right, initial.bottom) == (same.left, same.top, same.right, same.bottom)
    assert show_async(main, 6)
    wait_for(lambda: iconic(main), 'minimize')
    time.sleep(.35)
    move()
    print('Minimized window restored on requested display', flush=True)
    assert show_async(main, 3)
    wait_for(lambda: zoomed(main), 'maximize')
    time.sleep(.35)
    move()
    wait_for(lambda: zoomed(main), 'preserve maximized state')
    print('Maximized window retained state on requested display', flush=True)
    wait_for(lambda: log.read_text(encoding='utf-8').count('TRAY_MOVE ') == 4, 'four successful move actions')
finally:
    # Dismiss any test popup and ask this exact test instance to exit.
    for hwnd in menu_windows().values():
        post(hwnd, 0x0100, 0x1B, 0)
    post(tray, 0x0111, 2, 0)
    set_cursor(original_cursor.x, original_cursor.y)
