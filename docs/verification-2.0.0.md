# Rust / WinUI 3 v2.0.2 Verification

- Release source uses a pinned public naui revision (`f4d0e34b9b37bf6a0a047689f40eb782026c05ed`); Cargo.lock records the same commit.
- Cargo library regression tests: 13 passed, 0 failed.
- Owned HWND integration tests passed on the live two-monitor desktop: normal cross-monitor moves, zero writes when already at target, preserved maximized state, minimized-window restoration and destination proof, stable lock binding after explicit moves, passive minimized behavior, post-restore lock correction, and blacklist exclusion.
- Movement audit counters: normal cross-monitor move = one position write and zero state writes; already-target = zero position/state writes; minimized or maximized transfer = one position write and two state writes for restore/final state.
- Actual packaged WinUI manager launched from a copied app-local bundle with PATH restricted to Windows/System32. Runtime-class inspection confirmed `Microsoft.UI.Xaml.Window`; both WinUI runtime DLLs loaded from the package. A clean virtual machine was not tested.
- Packaged GUI was checked at simulated 800x520 and 640x440 work areas on the secondary monitor: window bounds remained in the selected monitor, icon resources were present, and tray restore worked.
- Native tray menu integration selected displays and moved the app-owned manager HWND to the selected monitor. Hidden and minimized cases restored to the target; maximized state was retained; selecting the current monitor did not change bounds. Only test instances used isolated data directories.
- The generated portable archive passed ZIP integrity and SHA-256 readback. Keep the whole extracted folder; the app-local WinUI runtime must remain beside the EXE.

Limitations: no clean-VM install test. Some applications, games, elevated windows, and custom-drawn surfaces may reject standard Win32 movement. Windows state-change animations can remain visible.
