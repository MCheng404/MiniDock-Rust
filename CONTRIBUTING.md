# 协作约定

这个仓库目前只有一个人开发，所以规则的目的**不是**流程管控，而是两件事：

1. 让"改坏了"在提交前暴露，而不是在下次跑起来时才发现；
2. 让历史可读 —— 半年后回来看 `git log`，能知道每次改动**为什么**发生。

技术栈的取舍见 [`DECISIONS.md`](DECISIONS.md)，模块职责见
[`ARCHITECTURE.md`](ARCHITECTURE.md)，这里只讲怎么提交。

---

## 一、克隆后必做一次

```bash
git config core.hooksPath .githooks   # 启用提交前门禁
```

不做这一步的话 `pre-commit` 不会跑，提交照样能过 —— 门禁是**约定**，
不是保险。另外确认 `core.autocrlf` 是 `false`（换行由 `.gitattributes`
决定，两者同时生效会打架）：

```bash
git config core.autocrlf false
```

## 二、门禁分三级

| 时机 | 命令 | 跑什么 | 耗时量级 |
|---|---|---|---|
| 每次提交（自动） | `.githooks/pre-commit` | `fmt --check` + `clippy -D warnings` | 秒 |
| 提交前 / 合并前 | `./scripts/build.sh --gate` | 上面两项 + `cargo test` | 十几秒 |
| 发版 | `./scripts/build.sh --release` | release 构建（LTO + `opt-level=z`） | 一分钟 |

`pre-commit` **故意不含测试**：全量测试要链接 7 个 crate 的测试二进制，
放进提交钩子的结果一定是被 `--no-verify` 绕过，那还不如不放。

跳过门禁的两种方式，语义不同：

```bash
SKIP_GATE=1 git commit ...   # 明确表示"这次不管"，钩子自己打印一行提示
git commit --no-verify ...   # git 原生逃生口，钩子完全不执行
```

## 三、分支

单人项目，不强制 Git Flow：

- **文档、单文件小修**：直接提 `main`。
- **跨 crate 改动、重构、改公共 API**：开分支，合回 `main` 时保留 merge
  commit（`git merge --no-ff`），这样 `git log --first-parent` 能看到
  "这是一整块工作"，而不是一串碎片。
- 分支名用 `类型/短描述`：`refactor/layout-dedup`、`fix/clock-offset`。

## 四、提交信息

格式：`类型: 一句话`（中文，不超过 72 字符，句末不加句号）。

| 类型 | 用于 |
|---|---|
| `feat` | 新功能（对应 PORTING.md 里的某个 P 项） |
| `fix` | 修缺陷。**正文里写清楚"原来错在哪"** |
| `refactor` | 不改行为的结构调整 |
| `perf` | 只改性能（附实测数字，见下） |
| `docs` | 只改文档 |
| `test` | 只改测试 |
| `chore` | 构建、依赖、仓库配置 |

正文（可选，但 `fix` / `perf` / `refactor` 建议写）回答三个问题：

```
fix: 时钟开启时栏体右端被裁

原因：calc_metrics 把 clock_extra 算进了逻辑宽度，但布局没减掉它。
影响：GDI+ 版与 GPU 版都偏右，宽度越窄越明显。
验证：cargo test -p minidock-core layout
```

性能类改动必须带**同机、同配置**的前后数字，否则下一个人没法判断
该不该保留。本仓库的做法是 `./scripts/probe-mem.sh` 与 `BUILD.md` 的实测表。

## 五、什么不该进版本库

| 路径 | 原因 |
|---|---|
| `target/` | 构建产物，GB 级 |
| `reference/` | 上游 C 源码。**版权未授予再分发权**（见 README），只允许本地对照 |
| `scratch/` | 临时验证程序（含编译出来的 exe） |
| `.workbuddy-ai/` | 协作工具的工作区数据，只对当前机器有意义 |
| `MiniDock.ini` / `icons/` | 运行期数据，便携模式下会写在这里 |

`Cargo.lock` **要提交**：这是二进制程序，不是库，依赖必须可复现。

## 六、文档与版本

- 每次改动都追加到 [`CHANGELOG.md`](CHANGELOG.md)，按里程碑分节；
  写"做了什么 / 为什么 / 已知问题"，**已知问题不要省**。
- 新的技术取舍追加到 [`DECISIONS.md`](DECISIONS.md) 的末尾，**不删旧条目** ——
  错误的决策留着，才有人知道为什么不能重犯。
- `crates/minidock-core/src/lib.rs` 的 `APP_VERSION` 与 crate 版本**分开**维护：
  前者跟 ini 兼容性走，后者跟内部重构走。
