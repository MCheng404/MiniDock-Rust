//! 计时器 / 秒表 / 待办：纯逻辑与 `[Tool]` 段的读写。
//!
//! 对应上游 v1.6.0 的 `ToolU` / `ToolLoad()` / `ToolSave()` / `ToolRemain()`
//! （minidock.c:6322-6370）。
//!
//! **时间基准由调用方传入**（毫秒整数），不在这里调 `Instant::now()`：
//! 这样所有时间相关行为都能用固定数字写单测，不必 sleep 等真实时间流逝。
//! 与 `core::layout` 的纯函数风格一致。

use crate::ini::Ini;

/// `[Tool]` 段名。
pub const SECTION: &str = "Tool";

/// 待办上限。上游写死 `todo[16]`（v1.6.0:6327）。
pub const MAX_TODOS: usize = 16;

/// 单条待办的文本上限。上游是 `WCHAR todo[16][96]`。
pub const MAX_TODO_LEN: usize = 96;

/// 倒计时默认总时长（毫秒）。上游 `TOOL_TOTAL_DEF = 5 * 60 * 1000`。
pub const DEFAULT_TOTAL_MS: i64 = 5 * 60 * 1000;

/// 计时器的两种模式。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ToolMode {
    /// 倒计时：从总时长往下数。
    #[default]
    Countdown,
    /// 秒表：从 0 往上数。
    Stopwatch,
}

impl ToolMode {
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Self::Countdown => 0,
            Self::Stopwatch => 1,
        }
    }

    #[must_use]
    pub const fn from_code(v: i32) -> Self {
        if v == 1 {
            Self::Stopwatch
        } else {
            Self::Countdown
        }
    }

    /// 面板上的切换按钮标题。
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Countdown => "计时",
            Self::Stopwatch => "秒表",
        }
    }
}

/// 一条待办。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Todo {
    pub text: String,
    pub done: bool,
}

impl Todo {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        let mut text = text.into();
        text.truncate(MAX_TODO_LEN);
        Self {
            text: text.trim().to_owned(),
            done: false,
        }
    }

    /// 序列化成 `[Tool]` 的一行：`{done}|{text}`。
    #[must_use]
    pub fn to_line(&self) -> String {
        format!("{}|{}", i32::from(self.done), self.text)
    }

    /// 解析一行。
    ///
    /// **修掉上游的 bug**：v1.6.0:6345 只在 `v[0] == '1'` 时跳过 `1|`
    /// 前缀，`0|` 时把前缀留在文本里。于是"存 → 读 → 存"每轮都会多叠
    /// 一层前缀（`0|买菜` → `0|0|买菜` → …），而上游每次打勾都会触发
    /// 一次存盘。
    ///
    /// 这里**只剥一层** `d|` 前缀，不循环剥。理由：
    ///
    /// - 循环剥会误伤合法文本 —— 用户写 "1|foo" 存盘后是 `0|1|foo`，
    ///   循环下去就变成 `foo`，把用户的内容吃掉了；
    /// - 只剥一层已经**根治累积**：被上游写坏的文件会残留一层 `0|`，
    ///   但从此不再继续恶化。
    #[must_use]
    pub fn parse_line(line: &str) -> Self {
        let rest = line.trim_end_matches(['\r', '\n']);
        // 第一段必须是 0 或 1 才算前缀（否则可能是文本里正好有 `x|`）。
        let (done, text) = match rest.split_once('|') {
            Some((head, tail)) => match head.trim() {
                "0" => (false, tail),
                "1" => (true, tail),
                _ => (false, rest),
            },
            None => (false, rest),
        };
        let mut text = text.to_owned();
        text.truncate(MAX_TODO_LEN);
        Self {
            text: text.trim().to_owned(),
            done,
        }
    }
}

/// 计时器 / 秒表 / 待办的全部状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolState {
    pub mode: ToolMode,
    pub todos: Vec<Todo>,
    /// 倒计时的总时长（毫秒）。秒表模式不用。
    pub total_ms: i64,
    /// 倒计时**暂停时**的剩余（毫秒）。运行中的剩余要减去已跑时间。
    remain_ms: i64,
    /// 秒表**暂停时**的累计（毫秒）。
    elapsed_ms: i64,
    /// 是否在跑。
    pub running: bool,
    /// 本段运行的起点（毫秒，调用方的时间基准）。
    start_ms: i64,
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            mode: ToolMode::Countdown,
            todos: Vec::new(),
            total_ms: DEFAULT_TOTAL_MS,
            remain_ms: DEFAULT_TOTAL_MS,
            elapsed_ms: 0,
            running: false,
            start_ms: 0,
        }
    }
}

impl ToolState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前该显示的毫秒数。
    ///
    /// 对应上游 `ToolRemain()`：跑着的时候减去/加上已跑时间。
    /// 倒计时不会返回负数。
    #[must_use]
    pub fn display_ms(&self, now_ms: i64) -> i64 {
        let run = if self.running {
            (now_ms - self.start_ms).max(0)
        } else {
            0
        };
        match self.mode {
            ToolMode::Countdown => (self.remain_ms - run).max(0),
            ToolMode::Stopwatch => self.elapsed_ms + run,
        }
    }

    /// 倒计时是否已经归零（且原本在跑）。
    #[must_use]
    pub fn is_finished(&self, now_ms: i64) -> bool {
        self.mode == ToolMode::Countdown && self.running && self.display_ms(now_ms) == 0
    }

    /// 推进一帧：倒计时归零时停表。
    ///
    /// 上游是在绘制里顺手停表（`ToolDraw` 里 `if (u->mode == 0 && u->running
    /// && t == 0) { running = 0; remainMs = 0; }`）。做成显式方法，
    /// 免得"画面没画就不停表"这种隐式依赖。
    pub fn tick(&mut self, now_ms: i64) {
        if self.is_finished(now_ms) {
            self.running = false;
            self.remain_ms = 0;
        }
    }

    /// 开始 / 继续。
    pub fn start(&mut self, now_ms: i64) {
        if self.running {
            return;
        }
        // 倒计时已经跑完时不重启 —— 否则点一下"开始"会立刻又归零。
        if self.mode == ToolMode::Countdown && self.remain_ms <= 0 {
            return;
        }
        self.running = true;
        self.start_ms = now_ms;
    }

    /// 暂停：把当前显示值固化下来。
    pub fn pause(&mut self, now_ms: i64) {
        if !self.running {
            return;
        }
        let cur = self.display_ms(now_ms);
        match self.mode {
            ToolMode::Countdown => self.remain_ms = cur,
            ToolMode::Stopwatch => self.elapsed_ms = cur,
        }
        self.running = false;
    }

    /// 开始 / 暂停切换（面板上的那个按钮）。
    pub fn toggle_run(&mut self, now_ms: i64) {
        if self.running {
            self.pause(now_ms);
        } else {
            self.start(now_ms);
        }
    }

    /// 重置：倒计时回到总时长，秒表归零。都会停表。
    pub fn reset(&mut self) {
        self.running = false;
        match self.mode {
            ToolMode::Countdown => self.remain_ms = self.total_ms,
            ToolMode::Stopwatch => self.elapsed_ms = 0,
        }
    }

    /// 切换模式。停表但**保留**各自的数值 —— 与上游一致
    /// （`mode` 与 `remainMs` / `stopMs` 是分开的字段，切回来值还在）。
    pub fn set_mode(&mut self, mode: ToolMode) {
        if self.mode == mode {
            return;
        }
        self.running = false;
        self.mode = mode;
    }

    /// 加一条待办。满了返回 `false`。空文本也返回 `false`。
    pub fn add_todo(&mut self, text: &str) -> bool {
        let todo = Todo::new(text);
        if todo.text.is_empty() || self.todos.len() >= MAX_TODOS {
            return false;
        }
        self.todos.push(todo);
        true
    }

    /// 勾选 / 取消勾选。下标越界返回 `false`。
    pub fn toggle_todo(&mut self, i: usize) -> bool {
        match self.todos.get_mut(i) {
            Some(t) => {
                t.done = !t.done;
                true
            }
            None => false,
        }
    }

    /// 删除一条。下标越界返回 `false`。
    pub fn remove_todo(&mut self, i: usize) -> bool {
        if i >= self.todos.len() {
            return false;
        }
        self.todos.remove(i);
        true
    }

    /// 未完成条数（条带卡片上显示的小圆点用）。
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.todos.iter().filter(|t| !t.done).count()
    }

    /// 从 `[Tool]` 段读出。缺键按默认值。
    #[must_use]
    pub fn load(ini: &Ini) -> Self {
        let mut out = Self::default();
        let n = ini
            .get_i32(SECTION, "N")
            .unwrap_or(0)
            .clamp(0, MAX_TODOS as i32);
        for i in 0..n {
            let line = ini.get(SECTION, &format!("T{i}")).unwrap_or_default();
            if line.is_empty() {
                continue;
            }
            out.todos.push(Todo::parse_line(line));
        }
        // 上游把秒写进 ini（`Total` / `Remain`），读出来再乘回毫秒。
        if let Some(secs) = ini.get_i32(SECTION, "Total") {
            out.total_ms = i64::from(secs.max(1)) * 1000;
        }
        out.remain_ms = ini
            .get_i32(SECTION, "Remain")
            .map_or(out.total_ms, |s| i64::from(s.max(0)) * 1000);
        out.elapsed_ms = 0;
        out.running = false;
        out
    }

    /// 写回 `[Tool]` 段。
    ///
    /// 条数变少时**要把多余的 `T{i}` 删掉**，否则文件里会留下旧条目，
    /// 下次读出条数虽然按 `N` 截断，但人看文件会以为数据没删干净。
    pub fn save(&self, ini: &mut Ini, now_ms: i64) {
        let old_n = ini.get_i32(SECTION, "N").unwrap_or(0).max(0);
        ini.set_i32(SECTION, "N", self.todos.len() as i32);
        for (i, todo) in self.todos.iter().enumerate() {
            ini.set(SECTION, &format!("T{i}"), todo.to_line());
        }
        for i in self.todos.len() as i32..old_n {
            ini.remove_key(SECTION, &format!("T{i}"));
        }
        ini.set_i32(SECTION, "Total", (self.total_ms / 1000) as i32);
        ini.set_i32(SECTION, "Remain", (self.display_ms(now_ms) / 1000) as i32);
    }
}

/// 把毫秒格式化成面板中央的大字。
///
/// 秒表带两位小数（上游 `%02d:%02d.%02d`），倒计时只到秒。
#[must_use]
pub fn format_ms(ms: i64, mode: ToolMode) -> String {
    let ms = ms.max(0);
    match mode {
        ToolMode::Countdown => format!("{:02}:{:02}", ms / 60_000, (ms / 1000) % 60),
        ToolMode::Stopwatch => format!(
            "{:02}:{:02}.{:02}",
            ms / 60_000,
            (ms / 1000) % 60,
            (ms % 1000) / 10
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todo_line_roundtrip() {
        let t = Todo {
            text: "买菜".to_owned(),
            done: true,
        };
        assert_eq!(t.to_line(), "1|买菜");
        assert_eq!(Todo::parse_line("1|买菜"), t);

        let t = Todo {
            text: "写代码".to_owned(),
            done: false,
        };
        assert_eq!(t.to_line(), "0|写代码");
        assert_eq!(Todo::parse_line("0|写代码"), t);
    }

    /// 上游 bug 的回归测试：`0|` 前缀必须被剥掉，否则每轮存读都多叠一层。
    #[test]
    fn todo_parse_strips_zero_prefix() {
        assert!(!Todo::parse_line("0|买菜").done);
        assert_eq!(Todo::parse_line("0|买菜").text, "买菜");
        // 被上游写坏的文件（双重前缀）：剥一层后残留一层，但不再累积。
        assert!(!Todo::parse_line("0|0|买菜").done);
        assert_eq!(Todo::parse_line("0|0|买菜").text, "0|买菜");
        // 关键：再存一次不会再叠。
        let once = Todo::parse_line("0|0|买菜");
        assert_eq!(once.to_line(), "0|0|买菜", "存量不再增长");
    }

    /// 循环剥前缀会误伤合法文本 —— 用户写 "1|foo" 必须原样保留。
    #[test]
    fn todo_parse_does_not_eat_user_text() {
        let t = Todo {
            text: "1|foo".to_owned(),
            done: false,
        };
        assert_eq!(t.to_line(), "0|1|foo");
        let back = Todo::parse_line("0|1|foo");
        assert!(!back.done);
        assert_eq!(back.text, "1|foo", "用户文本不能被当成前缀");
    }

    /// 文本里正好有 `x|` 时不能误吃。
    #[test]
    fn todo_parse_keeps_pipes_in_text() {
        assert_eq!(Todo::parse_line("a|b").text, "a|b");
        assert_eq!(Todo::parse_line("2|x").text, "2|x");
    }

    #[test]
    fn countdown_goes_down_while_running() {
        let mut s = ToolState::new();
        s.total_ms = 60_000;
        s.reset();
        assert_eq!(s.display_ms(0), 60_000);

        s.start(1_000);
        assert_eq!(s.display_ms(1_000), 60_000);
        assert_eq!(s.display_ms(11_000), 50_000);

        s.pause(11_000);
        assert_eq!(s.display_ms(99_999), 50_000, "暂停后不再走");
        s.start(20_000);
        assert_eq!(s.display_ms(25_000), 45_000);
    }

    #[test]
    fn countdown_stops_at_zero() {
        let mut s = ToolState::new();
        s.total_ms = 2_000;
        s.reset();
        s.start(0);
        assert_eq!(s.display_ms(5_000), 0, "不会变成负数");
        assert!(s.is_finished(5_000));
        s.tick(5_000);
        assert!(!s.running, "归零后应当停表");
        // 跑完再点开始不应当重新启动。
        s.start(6_000);
        assert!(!s.running);
    }

    #[test]
    fn stopwatch_goes_up() {
        let mut s = ToolState::new();
        s.set_mode(ToolMode::Stopwatch);
        s.reset();
        s.start(1_000);
        assert_eq!(s.display_ms(1_000), 0);
        assert_eq!(s.display_ms(3_500), 2_500);
        s.pause(3_500);
        assert_eq!(s.display_ms(9_000), 2_500);
        s.start(9_000);
        assert_eq!(s.display_ms(10_000), 3_500);
    }

    /// 切模式要停表，但两边的数值各自保留。
    #[test]
    fn mode_switch_keeps_each_value() {
        let mut s = ToolState::new();
        s.total_ms = 10_000;
        s.reset();
        s.start(0);
        s.pause(4_000);
        assert_eq!(s.display_ms(4_000), 6_000);

        s.set_mode(ToolMode::Stopwatch);
        assert!(!s.running);
        s.start(4_000);
        s.pause(7_000);
        assert_eq!(s.display_ms(7_000), 3_000);

        // 切回倒计时，之前剩的 6 秒还在。
        s.set_mode(ToolMode::Countdown);
        assert_eq!(s.display_ms(7_000), 6_000);
    }

    #[test]
    fn reset_restores_starting_value() {
        let mut s = ToolState::new();
        s.total_ms = 30_000;
        s.reset();
        s.start(0);
        s.pause(5_000);
        s.reset();
        assert_eq!(s.display_ms(5_000), 30_000);
        assert!(!s.running);

        s.set_mode(ToolMode::Stopwatch);
        s.start(0);
        s.pause(1_500);
        s.reset();
        assert_eq!(s.display_ms(1_500), 0);
    }

    #[test]
    fn todos_are_bounded_and_editable() {
        let mut s = ToolState::new();
        assert!(s.add_todo("  第一条  "));
        assert_eq!(s.todos[0].text, "第一条", "两端空白应当去掉");
        assert!(!s.add_todo("   "), "空文本不该进去");

        for i in 1..MAX_TODOS {
            assert!(s.add_todo(&format!("第 {i} 条")));
        }
        assert_eq!(s.todos.len(), MAX_TODOS);
        assert!(!s.add_todo("再来一条"), "超过上限要拒绝");

        assert!(s.toggle_todo(0));
        assert!(s.todos[0].done);
        assert_eq!(s.pending_count(), MAX_TODOS - 1);

        assert!(s.remove_todo(0));
        assert_eq!(s.todos.len(), MAX_TODOS - 1);
        assert!(!s.remove_todo(999), "越界要返回 false");
    }

    #[test]
    fn tool_section_roundtrip() {
        let mut s = ToolState::new();
        s.total_ms = 90_000;
        s.reset();
        s.add_todo("买菜");
        s.add_todo("写代码");
        s.toggle_todo(1);
        s.start(0);
        s.pause(30_000);

        let mut ini = Ini::new();
        s.save(&mut ini, 30_000);

        assert_eq!(ini.get(SECTION, "N"), Some("2"));
        assert_eq!(ini.get(SECTION, "T0"), Some("0|买菜"));
        assert_eq!(ini.get(SECTION, "T1"), Some("1|写代码"));
        assert_eq!(ini.get(SECTION, "Total"), Some("90"));
        assert_eq!(ini.get(SECTION, "Remain"), Some("60"));

        let back = ToolState::load(&ini);
        assert_eq!(back.todos, s.todos);
        assert_eq!(back.total_ms, 90_000);
        assert_eq!(back.display_ms(0), 60_000);
        // 运行状态不落盘：重启后一律是停的。
        assert!(!back.running);
    }

    /// 删条目后必须把多余的 `T{i}` 从文件里清掉。
    #[test]
    fn save_drops_stale_todo_keys() {
        let mut s = ToolState::new();
        s.add_todo("a");
        s.add_todo("b");
        s.add_todo("c");
        let mut ini = Ini::new();
        s.save(&mut ini, 0);
        assert_eq!(ini.get(SECTION, "T2"), Some("0|c"));

        s.remove_todo(2);
        s.remove_todo(1);
        s.save(&mut ini, 0);
        assert_eq!(ini.get(SECTION, "N"), Some("1"));
        assert_eq!(ini.get(SECTION, "T0"), Some("0|a"));
        assert!(ini.get(SECTION, "T1").is_none(), "旧键要删掉");
        assert!(ini.get(SECTION, "T2").is_none(), "旧键要删掉");
    }

    #[test]
    fn format_matches_upstream() {
        assert_eq!(format_ms(0, ToolMode::Countdown), "00:00");
        assert_eq!(format_ms(65_000, ToolMode::Countdown), "01:05");
        assert_eq!(format_ms(65_430, ToolMode::Stopwatch), "01:05.43");
    }
}
