// Emit a standard Win32 .res file directly so MSVC and rust-lld can link
// the icon even when the SDK resource compiler is absent.
use std::{env, fs, path::PathBuf};

fn resource(out: &mut Vec<u8>, kind: u16, id: u16, data: &[u8]) {
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(32u32.to_le_bytes());
    for value in [0xffffu16, kind, 0xffff, id] {
        out.extend(value.to_le_bytes());
    }
    out.extend([0u8; 16]);
    out.extend(data);
    while out.len() % 4 != 0 { out.push(0); }
}

fn main() {
    println!("cargo:rerun-if-changed=assets/window-lock-master.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") { return; }
    let ico = fs::read("assets/window-lock-master.ico").expect("read multi-size icon");
    assert_eq!(&ico[..4], &[0, 0, 1, 0]);
    let count = u16::from_le_bytes(ico[4..6].try_into().unwrap());
    let mut res = Vec::new();
    let mut group = ico[..6].to_vec();
    resource(&mut res, 0, 0, &[]);
    for (index, entry) in ico[6..6 + usize::from(count) * 16].chunks_exact(16).enumerate() {
        let size = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        let id = index as u16 + 1;
        resource(&mut res, 3, id, &ico[offset..offset + size]);
        group.extend(&entry[..12]);
        group.extend(id.to_le_bytes());
    }
    resource(&mut res, 14, 101, &group);
    let path = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("app-icon.res");
    fs::write(&path, res).expect("write app icon resource");
    for bin in ["WindowLockMasterRust", "WindowLockMasterTrayProbe"] {
        println!("cargo:rustc-link-arg-bin={bin}={}", path.display());
    }
}
