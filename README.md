# MiniDock-rs
Rust Rewrite of Windows Desktop Dock: GPU Rendering (wgpu / D3D12) + CPU Multi-threaded Pipeline
UI Logic Decoupled from Rendering Backend, Settings Interface Can Be Headless Driven by Kittest
---
## Current Status
**M0 (Spike) Completed** - It's not about making a functional Dock, but proving the technical premise first.
Conclusion: **Three Are Valid, One Is Disproven**.
| Acceptance Items | Status |
|---|---|
| D3D12 Device Available, egui Frame Output | ✅ `Dx12 / AMD Radeon 610M / Bgra8UnormSrgb` |
| egui Chinese Interface Drawn Normally | ✅ 9 Tabs, Check Boxes, Drop-down Boxes All Correct |
| Settings Interface Can Be Headless Tested | ✅ `cargo test -p minidock-settings` 5/5 |
| Window Transparency (Per-pixel Alpha) | ✅ Off-screen Rendering + `UpdateLayeredWindow` (Swap Chain Scheme Disproven) |
| Per-pixel Hit | ✅ Layered Window Judged by Alpha |
| Graphics Card Selection (Default Automatic) | ✅ `[Graphics] Adapter`, Matched by Name, Fallback on Mismatch |
| Resident Memory | ✅ Settings Window 111 MB (271 MB Before Optimization), See `BUILD.md` |
The DX12 / Vulkan Surface of wgpu Only Reports `Opaque`. For Transparency, Switch to "Off-screen Rendering + Read-back + `UpdateLayeredWindow"`, with About 1-6 MB Copy per Frame; Zero-copy Scheme (DirectComposition) Left to M3 - See [`DECISIONS.md`](DECISIONS.md) ADR-008.
The Dock Body (Icon, Zooming, Dragging, Grouping, Preview) **Has Not Been Migrated**; See M1 - M4 in [`PORTING.md`](PORTING.md).
## Quick Start
```bash
source scripts/env.sh          # Inject MSVC + Windows SDK
cargo build                    # Default Target: x86_64-pc-windows-msvc
cargo test  --workspace        # Logic Layer + Settings Interface (No Need for Monitor / GPU)
./scripts/build.sh --gate      # Full Gate Check: fmt + clippy + Test (Run This Before Submission)
# M0 Spike: Transparent GPU Window + Settings Interface, Exit with Esc
./scripts/build.sh --run -p minidock-gpu --bin minidock-spike
# Memory Probe: Compare Each Backend on the Same Graphics Card (Table See Section 5 of BUILD.md)
./scripts/probe-mem.sh
PROBE_LIST=1./target/x86_64-pc-windows-msvc/release/probe.exe   # First Check Adapter List
```
**After Cloning, Do It Once** `git config core.hooksPath.githooks`, Otherwise the `fmt` / `clippy` Gate Check Before Submission Will Not Take Effect. See [`CONTRIBUTING.md`](CONTRIBUTING.md) for Submission Information Format and Branch Convention.
Dependency Versions: `egui 0.37` / `egui-wgpu 0.37` / `wgpu 30` / `windows 0.62`, All Taking the Latest at Present.
