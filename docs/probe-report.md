# Rust / genuine WinUI 3 feasibility verification

Status: technical probe passed; production manager is still in development.

- Toolchain: isolated Rust 1.100.0 MSVC target, Rust bundled rust-lld, xwin 0.10.0 SDK/CRT. Global PATH unchanged.
- UI dependency: naui 0.18.0 source at f4d0e34b9b37bf6a0a047689f40eb782026c05ed; genuine Microsoft.UI.Xaml projection.
- Framework run: exit 0, actual runtime classes Microsoft.UI.Xaml.Window and Microsoft.UI.Xaml.Controls.ListView; DLL loaded from system WindowsApps runtime.
- Portable run: build with portable feature, copy EXE plus entire naui-winappsdk bundle to artifacts/portable-probe-copy. Run with PATH restricted to Windows/System32. Exit 0; both Microsoft.UI.Xaml.dll and Microsoft.WindowsAppRuntime.dll loaded from the copied package directory.
- Portable inventory at first check: 212 files / 60,231,946 bytes, includes probe-data log; EXE SHA-256 2fb8e9f9e3ca906a3abc32356671942f701206c6bf6a9a4ec1996b902c03eeab.
- Real UI interaction: accessibility Invoke on 验证按钮事件 changed visible label to 按钮事件已到达 Rust UI 线程; readback log contains BUTTON_CLICK verified. Closed exact fresh probe via its Close button; log EXIT success.
- No clean VM without preinstalled runtime tested; no claim of clean-machine validation. Module path proof verifies local runtime selection only.
- No target window movement/locking is implemented in this probe. Three example list rows are explicitly labeled test content and never described as real enumerated windows.

Reproduce: scripts/build-probe.ps1 -Portable; scripts/verify-portable-probe.py.

Build lessons: use rust-lld.exe entry rather than gcc-ld/lld-link.exe when Rust adds -flavor link. NuGet flatcontainer transfers stalled; official v2 endpoint with C:/Windows/System32/curl.exe completed all four packages and SHA-512 verification. Use explicit native paths for downloads, never assume TMPDIR agrees across native programs and background shell.

Production implementation underway: src/main.xaml and src/ui.rs; backend and tray modules delegated on separate file ownership. Preserve independent checkboxes and existing controls. Integration must compile and run before any final delivery claim.
