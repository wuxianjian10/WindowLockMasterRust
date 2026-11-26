import concurrent.futures, hashlib, pathlib, re, subprocess
root = pathlib.Path(__file__).resolve().parent.parent
text = (root/'research/naui/crates/naui-winui3/build/packages.rs').read_text()
items = re.findall(r'id: "([^"]+)",\s*version: "([^"]+)",\s*sha512: "([^"]+)"', text)
out=root/'toolchain/nuget'; out.mkdir(exist_ok=True)
def get(item):
    name, ver, expected=item; path=out/f'{name}.{ver}.nupkg'
    if not path.exists() or hashlib.sha512(path.read_bytes()).hexdigest()!=expected:
        subprocess.run(['C:/Windows/System32/curl.exe','--fail','--location','--connect-timeout','15','--max-time','180','--retry','1','-o',str(path),f'https://www.nuget.org/api/v2/package/{name}/{ver}'],check=True)
    actual=hashlib.sha512(path.read_bytes()).hexdigest()
    assert actual==expected, (name,actual,expected)
    return f'{path.name}: verified SHA-512 ({path.stat().st_size} bytes)'
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as ex:
    for result in ex.map(get,items): print(result,flush=True)
