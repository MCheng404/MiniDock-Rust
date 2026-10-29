# MiniDock-rs

Windows 桌面 Dock 的 **Rust 重写**：GPU 渲染（wgpu / D3D12）+ CPU 多线程管线，
UI 逻辑与渲染后端解耦，设置界面可被 kittest 无头驱动。

> 上游 C 版 `minidock.c`（4799 行，Win32 + GDI+）是**行为基准**，
> 不是要超越的对象。像素级对齐、配置格式兼容、崩溃模式修复，三件事优先于新功能。

---

## 当前状态

**M0（尖刺）已完成**——不是在做一个能用的 Dock，而是先证明技术前提。
结论：**三条成立，一条被证伪**。

| 验收项 | 状态 |
|---|---|
| D3D12 设备可用、egui 出帧 | ✅ `Dx12 / AMD Radeon 610M / Bgra8UnormSrgb` |
| egui 中文界面正常绘制 | ✅ 9 页签、复选框、下拉框全部正确 |
| 设置界面可无头测试 | ✅ `cargo test -p minidock-settings` 5/5 |
| 窗口透明（逐像素 alpha） | ✅ 离屏渲染 + `UpdateLayeredWindow`（交换链方案已证伪） |
| 逐像素命中 | ✅ 分层窗口按 alpha 判定 |
| 显卡选择（默认自动） | ✅ `[Graphics] Adapter`，按名字匹配、失配回退 |
| 常驻内存 | ✅ 设置窗 111 MB（调优前 271 MB），见 `BUILD.md` |

wgpu 的 DX12/Vulkan 表面都只报 `Opaque`，透明改走"离屏渲染 + 回读 +
`UpdateLayeredWindow`"，每帧约 1–6 MB 拷贝；零拷贝方案（DirectComposition）
留到 M3 —— 见 [`DECISIONS.md`](DECISIONS.md) ADR-008。

Dock 本体（图标、放大、拖拽、分组、预览）**尚未迁移**，见 [`PORTING.md`](PORTING.md) 的 M1–M4。

## 快速开始

```bash
source scripts/env.sh          # 注入 MSVC + Windows SDK
cargo build                    # 默认 target: x86_64-pc-windows-msvc
cargo test  --workspace        # 逻辑层 + 设置界面（无需显示器/GPU）

# M0 尖刺：透明 GPU 窗口 + 设置界面，Esc 退出
./scripts/build.sh --run -p minidock-gpu --bin minidock-spike

# 内存探针：同一块显卡上比较各后端（表格见 BUILD.md 第五节）
./scripts/probe-mem.sh
PROBE_LIST=1 ./target/x86_64-pc-windows-msvc/release/probe.exe   # 先看适配器清单
```

依赖版本：`egui 0.37` / `egui-wgpu 0.37` / `wgpu 30` / `windows 0.62`，全部取当前最新。

## 目录结构

```
crates/
  minidock-core      纯逻辑：配置模型、76 键表、INI 编解码、布局数学、条目序列化
  minidock-settings  egui 设置界面（9 页）· 无 wgpu / 无 Win32 · kittest 覆盖
  minidock-gpu       wgpu 表面 + egui 提交 + Win32 输入与命中（M0 尖刺）
  minidock-platform  Win32 系统服务：路径、显示器、DPI、主题、注册表、单实例
  minidock-gfx       GDI+ 封装（**遗留**：M2 删除，仅作 fallback 参考）
  minidock-ui        GDI+ 版 Dock 骨架（**遗留**，不参与 GPU 路线）
  minidock           可执行文件（遗留 GDI+ 版本，M2 后切换为 GPU 版）
docs: README / ARCHITECTURE / BUILD / PORTING / CONFIG / DECISIONS / CHANGELOG
```

`minidock-core` **没有任何 Win32 依赖**，可在 Linux CI 上编译测试——配置与布局
这两块最容易出错的逻辑，反馈回路是秒级的。

## 关键设计

- **一个 Win32 消息循环**，不用 eframe。托盘、全局热键、AppBar 预留、逐像素命中
  都必须在 `WndProc` 里说话，让框架接管事件循环等于把这些能力交出去。
- **一个 wgpu Device**，Dock 与设置窗口共用，字体图集与纹理只上传一次。
- **逐像素命中用 `WM_NCHITTEST`**，不是轮询。系统每次点击都会问一次，
  按布局算完回答 `HTTRANSPARENT` 即可——GPU 窗口没有 `UpdateLayeredWindow`
  那种天生穿透，这是唯一可靠做法。
- **UI 层不产生副作用**：设置界面只修改 `Config` 并返回 `Command`，
  打开对话框/写注册表/增删条目都由主程序执行。

细节见 [`ARCHITECTURE.md`](ARCHITECTURE.md)，决策理由见 [`DECISIONS.md`](DECISIONS.md)。

## 许可证与版权

⚠️ **上游 `minidock.c` 的版权归原作者 Wangcy 所有，仓库未授予再分发许可。**
本重写是衍生作品，**在取得授权前不得分发**；当前仅作本地研究与迁移用。
`minidock-gfx` / `minidock-ui` 中的绘制逻辑与常量来自上游源码，同样受此约束。

本项目自身新增代码暂不声明开源许可证，待上游授权明确后再定。
上游的图标资源（`res/app.ico`）**未**复制进本仓库。
