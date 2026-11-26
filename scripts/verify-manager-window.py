import ctypes as c, ctypes.wintypes as w, json, sys, time
u=c.windll.user32
u.GetWindowThreadProcessId.argtypes=[w.HWND,c.POINTER(w.DWORD)]
u.GetClassNameW.argtypes=[w.HWND,w.LPWSTR,c.c_int]
u.GetWindowTextW.argtypes=[w.HWND,w.LPWSTR,c.c_int]
u.IsWindowVisible.argtypes=[w.HWND];u.IsIconic.argtypes=[w.HWND]
u.ShowWindow.argtypes=[w.HWND,c.c_int];u.PostMessageW.argtypes=[w.HWND,w.UINT,w.WPARAM,w.LPARAM]
u.SetWindowPos.argtypes=[w.HWND,w.HWND,c.c_int,c.c_int,c.c_int,c.c_int,w.UINT]
u.GetWindowRect.argtypes=[w.HWND,c.POINTER(w.RECT)]
windows=[]
@c.WINFUNCTYPE(w.BOOL,w.HWND,w.LPARAM)
def cb(h,_):
 t=c.create_unicode_buffer(2048);k=c.create_unicode_buffer(256);p=w.DWORD()
 u.GetWindowTextW(h,t,2048);u.GetClassNameW(h,k,256);u.GetWindowThreadProcessId(h,c.byref(p))
 if t.value=='WindowLockMaster Rust 2.0.0' or k.value=='WindowLockMasterTray':windows.append((h,p.value,t.value,k.value))
 return True
u.EnumWindows(cb,0)
print(json.dumps(windows))
main=next(v for v in windows if v[2]=='WindowLockMaster Rust 2.0.0');h,pid,_,_=main
tray=next(v for v in windows if v[1]==pid and v[3]=='WindowLockMasterTray')[0]
mode=sys.argv[1] if len(sys.argv)>1 else 'inspect'
if mode=='resize':u.SetWindowPos(h,None,120,120,900,700,0x14)
if mode=='tray':
 u.PostMessageW(h,0x10,0,0);time.sleep(.6);assert not u.IsWindowVisible(h),'close must hide'
 u.PostMessageW(tray,0x401,1,0x203);time.sleep(.8);assert u.IsWindowVisible(h) and not u.IsIconic(h),'tray restore failed'
 print('PASS hidden -> native tray double-click message -> visible normal')
if mode=='exit':u.PostMessageW(tray,0x111,2,0);time.sleep(1)
r=w.RECT();u.GetWindowRect(h,c.byref(r));print('window',pid,[r.left,r.top,r.right,r.bottom],'visible',u.IsWindowVisible(h),'iconic',u.IsIconic(h))
