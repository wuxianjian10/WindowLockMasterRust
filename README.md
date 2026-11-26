# WindowLockMaster Rust

Rust 2.0.2 / Microsoft WinUI 3 窗口管理器。主程序、配置和单实例标识与旧 Python 版独立。

## 使用

解压完整便携目录后运行 `WindowLockMasterRust.exe`。同目录的 DLL、语言资源和 manifest 是 WinUI 运行库，必须一起保留。单独复制 EXE 不构成完整便携版。

1. 在窗口列表独立勾选要操作的窗口。
2. 选择目标显示器，点击“转移勾选窗口”。
3. 锁定、解锁、解除最小化和取消任务均作用于已勾选窗口。
4. 右键列表行可操作当前窗口或已勾选范围。
5. 关闭主窗口隐藏到托盘；双击托盘打开，托盘右键退出。
6. 托盘右键选择「移动本程序窗口到显示器」，可按编号和分辨率选择目标屏幕，并将主窗口打开到该屏幕的工作区内。

窗口状态自动更新，已存在的列表行复用并按字段更新，避免每次刷新重建列表、丢失勾选和滚动位置。窄窗口将操作区改为纵向布局，内容可以滚动。窗口尺寸按 DPI 和显示器工作区计算。

主窗口默认在鼠标所在屏幕打开。工作区变小时在原屏内缩小，手动拖动仍可换屏。界面使用 WinUI 3 桌面亚克力材质，模糊效果随 Windows 透明效果设置变化。双屏锁图标内嵌于 EXE，并用于任务栏和托盘。

黑名单规则、鼠标范围和开机启动位于可展开设置区。黑名单窗口从全选及转移/锁定中排除。鼠标锁定区域包含任务栏。默认配置位于 `%LOCALAPPDATA%\WindowLockMasterRust`。

## 移动行为

控制线程统一处理窗口状态和任务；WinEvent 回调只投递事件。显式移动优先于被动锁定纠正。相同请求合并；已在目标显示器不重复写入位置。被动锁定不恢复最小化窗口；拖动过程中暂停纠正。显式移动需核对实际显示器并保留原最小化/最大化状态。

## 构建

```powershell
./scripts/build-manager.ps1 -Test
python ./scripts/package-manager.py
```

当前工程使用项目内 Rust MSVC 工具链、SDK/CRT 和 Cargo 缓存。`.cargo/config.toml` 记录当前机器的 SDK/linker 路径，迁移源码后应调整为自己的工具链路径。

WinUI 投影依赖 [naui](https://github.com/mokuichi147/naui) `f4d0e34b9b37bf6a0a047689f40eb782026c05ed`（MIT/Apache-2.0），源码位于 `research/naui`，构建前应获取该精确版本。便携运行库由官方 NuGet 包取得并校验上游 SHA-512。

## 验证范围

技术探针已验证真实 `Microsoft.UI.Xaml.Window` 和 app-local DLL 加载。主程序仍需以最终验证报告为准；技术探针成功不能代替移动测试或干净机器测试。测试专用窗口用于跨屏和状态恢复验证，不操作用户窗口。`--readonly` 为只读预览；`--smoke --data-dir <独立目录>` 为有界只读启动测试。
