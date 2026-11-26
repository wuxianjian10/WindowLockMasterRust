"""Package complete app-local WinUI runtime; verify a copied launch."""
import hashlib,json,os,pathlib,shutil,subprocess,zipfile,datetime,sys
import tomllib
root=pathlib.Path(__file__).resolve().parent.parent
source=root/'target/release'
version=tomllib.loads((root/'Cargo.toml').read_text(encoding='utf-8'))['package']['version']
stamp=datetime.datetime.now().strftime('%Y%m%d-%H%M%S')
out=root/'artifacts'/f'WindowLockMasterRust-{version}-{stamp}'
out.mkdir(parents=True)
runtime=source/'naui-winappsdk'
assert (runtime/'naui-winappsdk.manifest').is_file()
for p in runtime.rglob('*'):
    if p.is_file():
        target=out/p.relative_to(runtime);target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(p,target)
shutil.copy2(source/'WindowLockMasterRust.exe',out/'WindowLockMasterRust.exe')
readme=f'''WindowLockMaster Rust {version}

解压整个目录后运行 WindowLockMasterRust.exe。同目录包含程序所需的 WinUI 3 运行库。
主窗口默认在鼠标所在屏幕打开。显示器工作区缩小时，窗口在原屏内调整尺寸。可以手动拖到其他屏幕。
玻璃模糊程度随 Windows 透明效果设置变化。
关闭主窗口会隐藏到托盘。双击托盘图标打开窗口，右键菜单可退出程序。
托盘右键选择「移动本程序窗口到显示器」，可选择目标屏幕并打开主窗口。
配置目录为 %LOCALAPPDATA%\\WindowLockMasterRust。
只读预览可运行 WindowLockMasterRust.exe --readonly。
'''
(out/'使用说明.txt').write_text(readme,encoding='utf-8-sig')
skip_smoke='--skip-smoke' in sys.argv
if not skip_smoke:
    env=os.environ.copy();env['PATH']=str(pathlib.Path(os.environ.get('SystemRoot','C:/Windows'))/'System32')
    data=root/'artifacts'/f'verify-package-{stamp}'
    r=subprocess.run([str(out/'WindowLockMasterRust.exe'),'--smoke','--data-dir',str(data)],cwd=out,env=env,timeout=30)
    text=(data/'ui.log').read_text(encoding='utf-8')
    assert r.returncode==0,(r.returncode,text)
    assert 'UI_READY class=Microsoft.UI.Xaml.Window' in text,text
    assert 'UI_SMOKE_OK' in text and 'ERROR' not in text,text
    assert 'BACKDROP Microsoft.UI.Xaml.Media.DesktopAcrylicBackdrop' in text,text
    assert 'ICON resource=101' in text,text
    for name in ['Microsoft.UI.Xaml.dll','Microsoft.WindowsAppRuntime.dll']:
        line=next(l for l in text.splitlines() if l.startswith('MODULE '+name+'='))
        assert pathlib.Path(line.split('=',1)[1]).resolve().is_relative_to(out.resolve()),line
exe_hash=hashlib.sha256((out/'WindowLockMasterRust.exe').read_bytes()).hexdigest()
archive=out.parent/(out.name+'.zip')
with zipfile.ZipFile(archive,'w',zipfile.ZIP_DEFLATED) as z:
    for p in out.rglob('*'):
        if p.is_file():z.write(p,pathlib.Path(out.name)/p.relative_to(out))
with zipfile.ZipFile(archive) as z:assert z.testzip() is None
sha=hashlib.sha256(archive.read_bytes()).hexdigest()
(pathlib.Path(str(archive)+'.sha256')).write_text(f'{sha}  {archive.name}\n',encoding='ascii')
files=[p for p in out.rglob('*') if p.is_file()]
report={'version':version,'directory':str(out),'exe_sha256':exe_hash,'files':len(files),'bytes':sum(p.stat().st_size for p in files),'smoke_exit':None if skip_smoke else 0,'smoke_skipped':skip_smoke,'app_local_runtime':True,'clean_machine_tested':False}
report['zip']=str(archive);report['zip_sha256']=sha
(out/'verification.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
(root/'artifacts/latest-package.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
print(json.dumps(report,indent=2))
