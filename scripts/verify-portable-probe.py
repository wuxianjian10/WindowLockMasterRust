import hashlib, json, os, pathlib, shutil, subprocess, sys
root=pathlib.Path(__file__).resolve().parent.parent
profile=root/'target/release'
exe=profile/'WindowLockMasterRustProbe.exe'
bundle=profile/'naui-winappsdk'
assert exe.is_file(), exe
assert (bundle/'naui-winappsdk.manifest').is_file(), bundle
out=root/'artifacts/portable-probe-copy'
out.mkdir(parents=True,exist_ok=True)
for source in bundle.rglob('*'):
    if source.is_file():
        dest=out/source.relative_to(bundle); dest.parent.mkdir(parents=True,exist_ok=True); shutil.copy2(source,dest)
shutil.copy2(exe,out/exe.name)
env=os.environ.copy(); env['PATH']=os.environ.get('SystemRoot','C:/Windows')+'/System32'; env['WLM_PROBE_DATA']=str(out/'probe-data')
result=subprocess.run([str(out/exe.name),'--smoke'],cwd=out,env=env,timeout=30)
log=(out/'probe-data/probe.log').read_text(encoding='utf-8')
print(log)
assert result.returncode==0, result.returncode
assert 'CLASS window=Microsoft.UI.Xaml.Window' in log
assert 'CLASS list=Microsoft.UI.Xaml.Controls.ListView' in log
assert 'EXIT success' in log
for module in ('Microsoft.UI.Xaml.dll','Microsoft.WindowsAppRuntime.dll'):
    line=next(l for l in log.splitlines() if l.startswith('MODULE '+module+'='))
    path=pathlib.Path(line.split('=',1)[1]).resolve()
    assert path.is_relative_to(out.resolve()), line
files=[p for p in out.rglob('*') if p.is_file()]
report={'stage':'technical probe only','exe':str(out/exe.name),'sha256':hashlib.sha256((out/exe.name).read_bytes()).hexdigest(),'files':len(files),'total_bytes':sum(p.stat().st_size for p in files),'exit_code':result.returncode,'app_local_runtime_verified':True,'clean_machine_verified':False}
(out/'verification.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
print(json.dumps(report,indent=2))
