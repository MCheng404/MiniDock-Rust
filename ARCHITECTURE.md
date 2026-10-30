# 架构

## 一、三层，两个循环

```
┌─ UI 线程 ────────────────────────────────────────────────┐
│ Win32 消息循环（托盘/热键/AppBar/NCHITTEST）              │
│   ↓ 事件                                                  │
│ Dock 状态（ArcSwap<Snapshot> 只读快照）                   │
│   ↓                                                       │
│ egui::Context::run_ui → tessellate → egui-wgpu → 交换链   │
└───────────────────────────────────────────────────────────┘
        ↑ 纹理/快照                    ↑ 磨砂纹理
┌─ CPU 工作线程 ──────────┐   ┌─ GPU ──────────────────────┐
│ rayon：图标解码/缩放/预乘 │   │ DXGI Desktop Duplication   │
│ IO：窗口扫描/favicon/缩略图│   │  → compute 两趟模糊        │
└──────────────────────────┘   └────────────────────────────┘
```

**只有一个消息循环**，且在 UI 线程。CPU 工作线程不创建窗口、不碰 `wgpu::Device`；
它们只生产"可以上传的数据"，由 UI 线程在提交帧之前完成纹理上传。

## 二、crate 依赖

```
minidock-core        （无 Win32 依赖，纯逻辑）
   ↑        ↑
   │        └── minidock-settings （egui，无 wgpu）── kittest 测试
   │                    ↑
   └── minidock-platform│
              ↑         │
         minidock-gpu ──┘  （wgpu + Win32 窗口/输入/命中）
```

依赖方向是单向的：`gpu → settings → core`、`gpu → platform`。
`settings` 不认识 `wgpu`，所以它的测试不需要 GPU/显示器；
`core` 不认识 Windows，所以它的测试能跑在任何宿主上。

## 三、渲染管线

一帧的完整路径在 **`frame.rs::FrameRenderer::render`** 里，设置窗口
（`overlay.rs`）与 Dock 窗口（`bin/dock.rs`）共用同一份实现 —— 这条链路
原先两边各写一遍，逐行相同，任何 API 变动都要改两处。

1. 调用方组装 `egui::RawInput`（含 `screen_rect`、`time`、`viewports`），
   传给 `render(raw, hwnd, pos, |ui| ...)`。
2. `ctx.run_ui(raw, build)`：闭包里只负责画。
3. `ctx.tessellate(shapes, ppp)` → `Vec<ClippedPrimitive>`。
4. `textures_delta.set` 逐项 `renderer.update_texture`（字形图集、图标纹理）。
5. `renderer.update_buffers` 写顶点/索引缓冲。
6. `begin_render_pass`，`LoadOp::Clear(Color::TRANSPARENT)` —— **透明清屏是窗口能透出桌面的关键**。
7. `renderer.render(&mut pass, ...)`，`queue.submit`。
8. `copy_texture_to_buffer` → 同步回读 → 预乘 BGRA 写入 DIB → `UpdateLayeredWindow`。
9. `textures_delta.free` 回收，然后 `clear()`（不 clear 会在 Drop 时断言 panic）。

返回 `FrameOutcome { cursor, repaint_delay }` —— 调用方只需要这两个值。

### 槽位布局：`core::layout`

"每个图标画在哪、多大、鼠标命中谁"全部是纯函数，住在 `core::layout`：

| 函数 | 职责 |
|---|---|
| `calc_metrics` | 由配置 + 工作区算出 `Metrics`（图标边长、栏高、时钟占位……） |
| `base_centers` | 未放大时的槽位中心（放大权重必须以它为基准） |
| `magnify_targets` | 本帧每个槽位的目标边长（余弦衰减） |
| `lerp_to` | 向目标值指数逼近一帧，返回"还需不需要下一帧" |
| `place_slots` | 放大后的实际槽位 `bar_l/bar_w/slot_l/slot_r/act_c` |
| `bars` | 纵向基线 `bar_top/bar_bottom/icon_bottom` |
| `hit_test` | 命中下标（从左到右第一个，见下） |

GPU 版与 GDI+ 版都只是"取结果 → 画"。这套逻辑曾经两边各写一遍，
已经漂移过（GDI+ 版拿动画中的 `cur` 去算基准宽度，导致基准槽位每帧移动）。

## 四、帧调度：空闲不提交

```rust
loop {
    if ctx.has_requested_repaint() { render()?; sleep(8ms); continue; }
    GetMessageW(...)   // 阻塞，直到有消息
    TranslateMessage / DispatchMessage
}
```

egui 只在有动画、悬停、输入时请求重绘；其余时间线程阻塞在 `GetMessageW`，
CPU 占用为 0。这条性质是产品承诺（"空闲无定时器"），**不能被任何"为了流畅"
的常驻 vsync 循环破坏**。M1 引入的动画必须走 `ctx.request_repaint()`，
不允许自建 60fps 定时器。

## 五、输入与命中

- 鼠标/滚轮/键盘由 `input.rs` 翻译成 `egui::Event`。这份翻译是"不用 eframe"
  的代价，M0 只覆盖鼠标、滚轮、Esc/Enter/Tab/方向键。
- **命中测试在 `WM_NCHITTEST` 里同步完成**：把光标坐标换成逻辑像素，
  判断是否落在面板矩形内，是则 `HTCLIENT`，否则 `HTTRANSPARENT`。
  系统每次点击都会问，不需要轮询、不存在"鸡生蛋"问题。
- M1 要补：按圆角/图标实际形状判定（现在是矩形近似）、`WM_MOUSELEAVE`、
  IME（`WM_IME_*` → `egui::Event::Ime`），以及输入框的原生 `EDIT` 兜底。

## 六、线程模型

| 线程 | 职责 | 禁止事项 |
|---|---|---|
| UI | 消息泵、egui、GPU 提交、命中 | 不做磁盘 IO、不枚举窗口 |
| rayon 池 | 图标解码/缩放/预乘、模糊 fallback | 不碰窗口/device |
| IO 线程 | 窗口扫描、favicon 下载、缩略图抓取 | 不产生 `wgpu` 对象 |
| GPU | 桌面复制、compute 模糊 | 不阻塞等待 CPU 结果 |

跨线程数据用 `ArcSwap`（快照，读多写少）+ `crossbeam-channel`（任务与结果）。
不用 tokio：这里没有大量并发连接，只有几条长驻线程 + 一个线程池，
async 运行时带来的调度开销与栈占用都不划算。

## 七、不变式（违反即 bug）

1. **字体先于第一帧**。egui 0.37 缺字形直接 `panic`（`MissingGlyphPolicy::Panic`），
   不是画豆腐块。`fonts::install_cjk` 必须在 `run_ui` 之前调用。
2. **`GdiPlus` 先于所有 GDI+ 对象构造、后于它们析构**（遗留路径）。
   Rust 按声明逆序析构，`main` 里把 `_gp` 写在 `Dock` 前面即可。
3. **设置界面不产生副作用**。它只能改 `Config` 并返回 `Command`；
   文件、注册表、对话框由主程序执行。这条保证了 kittest 的可断言性。
4. **一个 Device**。禁止为第二个窗口再建 device —— 纹理与图集无法共享，
   内存会翻倍。
5. **配置写入只有一个出口**。所有值必须经 `Config::apply_i32`（内含夹紧），
   不允许直接改字段。原版崩溃的一半来自"某处忘了夹紧"。

## 八、与 C 版的模块映射

| 原版（minidock.c） | Rust | 状态 |
|---|---|---|
| `LoadConfig` / `SaveSettingsOnly` / `g_tab[]` | `core::config` | ✅ 已迁移 + 夹紧 + 单测 |
| `ParseItemLine` / `SaveNow` / `LoadItems` | `core::item` / `core::store` | ✅ 已迁移 + 单测 |
| `CalcMetrics` / `MapR` / `ToLogical` / `HitIndex` | `core::layout` | ✅ 已迁移 + 单测 |
| `InitPaths` | `platform::paths` | ✅ |
| `GetMon` / `GetDeviceCaps` | `platform::display` / `win` | ✅ |
| `SystemDark` / `GetAutostart` | `platform::theme` | ✅ |
| `BuildRows` / `PaintSettings` / `SetProc` | `settings::pages` | ✅ M0 壳（9 页） |
| `RenderFrame` / `DrawBar` / `DrawBubbleAt` | `gpu::dock`（待写） | ⏳ M1 |
| `LoadItemIcon` / `HiconToGp` / `FitBitmap` | `platform::icons`（待写） | ⏳ M2 |
| `UpdateFrost` / `BlurPass` | `gpu::frost`（DXGI + compute） | ⏳ M3 |
| `DT_*`（OLE 拖放） | `platform::drop` | ⏳ M2 |
| `ShowMenu` / `ShowGroup` / `ShowFan` / `ShowPreview` | `gpu::popups` | ⏳ M3 |
| `SetMinHook` / `AnimStep` | `platform::minimize` | ⏳ M4 |

函数级台账与验收标准见 [`PORTING.md`](PORTING.md)。
