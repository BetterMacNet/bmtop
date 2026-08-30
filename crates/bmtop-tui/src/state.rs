//! 交互状态与按键处理。

use crate::widgets::{decaying_peak, push_bounded, DETAIL_SCROLL_STEP};
use bmtop_core::{AppMode, DiskVolume, Language, Strings, SystemSnapshot};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Search,
    UserFilter,
    /// top 风格的 `s<秒>`：进入后数字进提示符，不再触发 1-9 切页。
    Interval,
    Help,
    Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSignalKind {
    Terminate,
    Kill,
}

/// 进程表排序列。`o` 循环切换，`O` / `R` 反转升降序（对齐 top 的 o/O 心智）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Cpu,
    Gpu,
    /// 能耗影响（活动监视器口径）。
    Energy,
    /// 估算功耗（瓦特）。
    Power,
    Memory,
    Pid,
}

impl SortKey {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Cpu => Self::Gpu,
            Self::Gpu => Self::Energy,
            Self::Energy => Self::Power,
            Self::Power => Self::Memory,
            Self::Memory => Self::Pid,
            Self::Pid => Self::Cpu,
        }
    }
}

/// 硬件 / 传感器页左侧列表的一项，右侧展示 `body`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailSection {
    pub name: String,
    pub body: String,
}

impl DetailSection {
    pub fn new(name: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            body: body.into(),
        }
    }
}

/// 按模式懒加载的补充数据。采集这些要起子进程，所以只在切到对应模式时拉一次。
#[derive(Debug, Clone, PartialEq)]
pub enum ModeDetail {
    None,
    Disks(Vec<DiskVolume>),
    Sections(Vec<DetailSection>),
}

#[derive(Debug, Clone)]
pub(crate) struct PendingAction {
    pub(crate) pid: i32,
    pub(crate) start_seconds: u64,
    pub(crate) start_microseconds: u64,
    pub(crate) signal: ProcessSignalKind,
    pub(crate) confirmation: String,
}

#[derive(Debug, Clone)]
pub struct UiState {
    pub mode: AppMode,
    pub paused: bool,
    pub input_mode: InputMode,
    pub input: String,
    pub sort_key: SortKey,
    pub sort_descending: bool,
    /// 进程表的用户过滤（`u` 键），空串表示不过滤。
    pub user_filter: String,
    /// `s` 间隔提示符的输入缓冲。
    pub interval_input: String,
    /// `c`：命令列显示完整路径。
    pub show_full_path: bool,
    /// `i`：隐藏空闲进程（CPU 为 0 的行；首个样本 CPU 未知的行保留）。
    pub hide_idle: bool,
    /// `V`：树状视图（按 PPID 缩进）。
    pub tree_view: bool,
    /// `H`：详情栏切到选中进程的线程视图。
    pub thread_view: bool,
    pub selected: usize,
    pub status: String,
    pub snapshot: Option<SystemSnapshot>,
    /// 硬件 / 传感器页的分区列表。
    pub sections: Vec<DetailSection>,
    /// 左侧分区列表的游标。和 `selected`（进程行）互不干扰。
    pub section_selected: usize,
    /// 右侧详情的纵向滚动量，PgUp / PgDn 控制。
    pub detail_scroll: u16,
    /// 磁盘卷缓存。概览页和磁盘页都要用，切到硬件页时不能丢。
    pub disks: Vec<DiskVolume>,
    pub detail_error: Option<String>,
    /// 按下 `r` 的显式标志。原来靠比对状态栏文案判断，一翻译就失效了。
    pub refresh_requested: bool,
    /// 采样间隔，只用于标题栏显示。
    pub interval_millis: u64,
    pub language: Language,
    receive_history: VecDeque<f64>,
    send_history: VecDeque<f64>,
    cpu_history: VecDeque<f64>,
    memory_history: VecDeque<f64>,
    power_history: VecDeque<f64>,
    energy_history: VecDeque<f64>,
    receive_peak: f64,
    send_peak: f64,
    /// 屏幕 FPS 开关（`f` 键），随下一次采样请求带给采集器。
    pub fps_enabled: bool,
    pub(crate) pending_action: Option<PendingAction>,
    pub(crate) action_input: String,
    completed_action: Option<PendingAction>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            mode: AppMode::Overview,
            paused: false,
            input_mode: InputMode::Normal,
            input: String::new(),
            sort_key: SortKey::Cpu,
            sort_descending: true,
            user_filter: String::new(),
            interval_input: String::new(),
            show_full_path: false,
            hide_idle: false,
            tree_view: false,
            thread_view: false,
            selected: 0,
            status: Language::from_environment()
                .strings()
                .status_sampling
                .to_string(),
            snapshot: None,
            sections: Vec::new(),
            section_selected: 0,
            detail_scroll: 0,
            disks: Vec::new(),
            detail_error: None,
            refresh_requested: false,
            interval_millis: 1_000,
            language: Language::from_environment(),
            receive_history: VecDeque::new(),
            send_history: VecDeque::new(),
            cpu_history: VecDeque::new(),
            memory_history: VecDeque::new(),
            power_history: VecDeque::new(),
            energy_history: VecDeque::new(),
            receive_peak: 0.0,
            send_peak: 0.0,
            fps_enabled: false,
            pending_action: None,
            action_input: String::new(),
            completed_action: None,
        }
    }
}

impl UiState {
    pub fn with_mode(mode: AppMode) -> Self {
        Self {
            mode,
            ..Self::default()
        }
    }

    pub fn take_refresh_request(&mut self) -> bool {
        std::mem::take(&mut self.refresh_requested)
    }

    pub fn strings(&self) -> &'static Strings {
        self.language.strings()
    }

    /// 切换语言时状态栏那句话也要跟着换，否则会残留上一种语言。
    pub fn set_language(&mut self, language: Language) {
        self.language = language;
        self.status = if self.snapshot.is_some() {
            self.live_status()
        } else {
            self.strings().status_sampling
        }
        .to_string();
    }

    fn live_status(&self) -> &'static str {
        if self.paused {
            self.strings().status_paused
        } else {
            self.strings().status_live
        }
    }

    pub fn set_snapshot(&mut self, snapshot: SystemSnapshot) {
        if self.mode == AppMode::Gpu && snapshot.gpu.is_none() {
            self.switch_mode(AppMode::Overview);
        }
        self.record_history(&snapshot);
        // 光标钉住进程而不是行号：CPU 排序每拍都会重排，按行号保留会让
        // 光标每拍落到别的进程上，详情增强（fd/IO/线程）永远追不上正在看的行。
        let previous_pid = self.selected_process_pid();
        self.snapshot = Some(snapshot);
        self.status = self.live_status().to_string();
        let position = previous_pid.and_then(|pid| {
            self.filtered_processes()
                .iter()
                .position(|(_, process)| process.pid == pid)
        });
        self.selected = position.unwrap_or_else(|| {
            self.selected
                .min(self.filtered_processes().len().saturating_sub(1))
        });
    }

    /// 每次快照都记一笔走势。CPU 和内存的百分比条只说明「此刻」，
    /// 概览要看出变化就得有历史；网络速率没有分母，更是只能靠走势图。
    fn record_history(&mut self, snapshot: &SystemSnapshot) {
        push_bounded(
            &mut self.cpu_history,
            snapshot.cpu.total_percent.unwrap_or(0.0),
        );
        let memory = &snapshot.memory;
        let used_percent = if memory.total_bytes > 0 {
            memory.used_bytes as f64 / memory.total_bytes as f64 * 100.0
        } else {
            0.0
        };
        push_bounded(&mut self.memory_history, used_percent);
        // 功耗只在 SoC 快照存在时记录；缺口不补零，免得走势图画出假谷底。
        if let Some(watts) = snapshot
            .soc
            .as_ref()
            .and_then(|soc| soc.power.total_watts())
        {
            push_bounded(&mut self.power_history, watts);
        }
        // 能耗同理：首个采样每行都是 None，此时不记点而不是记 0。
        if let Some(total) = total_energy_impact(snapshot) {
            push_bounded(&mut self.energy_history, total);
        }
        self.record_network_rates(snapshot);
    }

    fn record_network_rates(&mut self, snapshot: &SystemSnapshot) {
        let total = |select: fn(&bmtop_core::NetworkInterfaceMetrics) -> Option<f64>| {
            snapshot
                .interfaces
                .iter()
                .filter_map(select)
                .filter(|value| value.is_finite())
                .sum::<f64>()
        };
        let receive = total(|interface| interface.receive_bytes_per_second);
        let send = total(|interface| interface.send_bytes_per_second);
        self.receive_peak = decaying_peak(self.receive_peak, receive);
        self.send_peak = decaying_peak(self.send_peak, send);
        push_bounded(&mut self.receive_history, receive);
        push_bounded(&mut self.send_history, send);
    }

    /// 网络收发的衰减峰值（字节/秒），网络页副标题用。
    pub fn network_peaks(&self) -> (f64, f64) {
        (self.receive_peak, self.send_peak)
    }

    pub fn receive_history(&self) -> &VecDeque<f64> {
        &self.receive_history
    }

    pub fn send_history(&self) -> &VecDeque<f64> {
        &self.send_history
    }

    pub fn cpu_history(&self) -> &VecDeque<f64> {
        &self.cpu_history
    }

    pub fn memory_history(&self) -> &VecDeque<f64> {
        &self.memory_history
    }

    pub fn power_history(&self) -> &VecDeque<f64> {
        &self.power_history
    }

    pub fn energy_history(&self) -> &VecDeque<f64> {
        &self.energy_history
    }

    /// 上下行合并的速率序列，概览里只放得下一条走势。
    pub fn network_history(&self) -> Vec<f64> {
        self.receive_history
            .iter()
            .zip(self.send_history.iter())
            .map(|(receive, send)| receive + send)
            .collect()
    }

    /// 把懒加载的结果并入状态。各类结果分开缓存，切模式不会互相冲掉。
    pub fn apply_detail(&mut self, detail: Result<ModeDetail, String>) {
        match detail {
            Ok(ModeDetail::None) => self.detail_error = None,
            Ok(ModeDetail::Disks(disks)) => {
                self.disks = disks;
                self.detail_error = None;
            }
            Ok(ModeDetail::Sections(sections)) => {
                self.sections = sections;
                self.section_selected = self
                    .section_selected
                    .min(self.sections.len().saturating_sub(1));
                self.detail_error = None;
            }
            Err(error) => self.detail_error = Some(error),
        }
    }

    pub fn selected_section(&self) -> Option<&DetailSection> {
        self.sections.get(self.section_selected)
    }

    /// 硬件和传感器共用「左列表 + 右详情」的布局与导航。
    pub(crate) fn uses_section_list(&self) -> bool {
        // 传感器页已改为快照实时页，分区列表只剩硬件页在用。
        matches!(self.mode, AppMode::Hardware)
    }

    fn gpu_available(&self) -> bool {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.gpu.as_ref())
            .is_some()
    }

    /// GPU 模式在没有 GPU 能力时不可进入，其余模式直接切换。
    fn request_mode(&mut self, mode: Option<AppMode>) {
        let Some(mode) = mode else { return };
        if mode == AppMode::Gpu && !self.gpu_available() {
            return;
        }
        self.switch_mode(mode);
    }

    /// `+`/`-` 运行时调采样间隔，钳在 250ms–60s。生效由主循环感知。
    fn adjust_interval(&mut self, delta_millis: i64) {
        let next = (self.interval_millis as i64 + delta_millis).clamp(
            bmtop_core::RefreshInterval::MIN_MILLIS as i64,
            bmtop_core::RefreshInterval::MAX_MILLIS as i64,
        );
        self.interval_millis = next as u64;
    }

    /// ←/→ 在模式条上循环切换，首尾相接；GPU 不可用时跳过（模式条也不显示它）。
    pub(crate) fn cycle_mode(&mut self, step: i8) {
        let modes = AppMode::ALL;
        let length = modes.len() as i8;
        let mut index = modes
            .iter()
            .position(|mode| *mode == self.mode)
            .unwrap_or(0) as i8;
        for _ in 0..length {
            index = (index + step).rem_euclid(length);
            let mode = modes[index as usize];
            if mode == AppMode::Gpu && !self.gpu_available() {
                continue;
            }
            self.switch_mode(mode);
            return;
        }
    }

    fn switch_mode(&mut self, mode: AppMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.section_selected = 0;
        self.detail_scroll = 0;
    }

    /// 当前选中进程的 PID，采样线程用它决定为谁补齐详情字段。
    pub fn selected_process_pid(&self) -> Option<i32> {
        self.filtered_processes()
            .get(self.selected)
            .map(|(_, process)| process.pid)
    }

    /// 返回 (树深度, 行)。非树状视图下深度恒为 0。
    pub fn filtered_processes(&self) -> Vec<(usize, &bmtop_core::ProcessRow)> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let query = self.input.to_ascii_lowercase();
        let user = self.user_filter.to_ascii_lowercase();
        let mut rows: Vec<_> = snapshot
            .processes
            .iter()
            .filter(|process| {
                (query.is_empty()
                    || process.name.to_ascii_lowercase().contains(&query)
                    || process.pid.to_string().contains(&query))
                    && (user.is_empty() || process.user.to_ascii_lowercase().contains(&user))
                    // CPU 未知（首个样本）的行保留，只藏确定为 0 的；
                    // 但唤醒频繁的进程 CPU 常年是 0.0 而能耗不低，
                    // 那正是能耗列要暴露的东西，不能被「隐藏空闲」抹掉。
                    && (!self.hide_idle
                        || process.cpu_percent.is_none_or(|value| value > 0.0)
                        || process.energy_impact.is_some_and(|value| value > 0.0))
            })
            .collect();
        // 降序是默认视角（大的在前）；PID 也遵守同一方向开关，行为可预期。
        rows.sort_by(|left, right| match self.sort_key {
            SortKey::Cpu => right
                .cpu_percent
                .partial_cmp(&left.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal),
            SortKey::Gpu => right
                .gpu_percent
                .unwrap_or(0.0)
                .partial_cmp(&left.gpu_percent.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal),
            SortKey::Energy => right
                .energy_impact
                .unwrap_or(0.0)
                .partial_cmp(&left.energy_impact.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal),
            SortKey::Power => right
                .power_watts
                .unwrap_or(0.0)
                .partial_cmp(&left.power_watts.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal),
            SortKey::Memory => right.resident_bytes.cmp(&left.resident_bytes),
            SortKey::Pid => right.pid.cmp(&left.pid),
        });
        if !self.sort_descending {
            rows.reverse();
        }
        if self.tree_view {
            tree_order(rows)
        } else {
            rows.into_iter().map(|row| (0, row)).collect()
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.input_mode == InputMode::Action {
            match key.code {
                KeyCode::Esc => {
                    self.pending_action = None;
                    self.action_input.clear();
                    self.input_mode = InputMode::Normal;
                    self.status = self.strings().status_action_cancel.to_string();
                }
                KeyCode::Enter => {
                    let valid = self
                        .pending_action
                        .as_ref()
                        .map(|action| {
                            action.signal == ProcessSignalKind::Terminate
                                && self.action_input.eq_ignore_ascii_case("y")
                                || action.signal == ProcessSignalKind::Kill
                                    && self.action_input == action.pid.to_string()
                        })
                        .unwrap_or(false);
                    if valid {
                        self.completed_action = self.pending_action.take();
                        self.action_input.clear();
                        self.input_mode = InputMode::Normal;
                    } else {
                        self.status = self.strings().status_action_mismatch.to_string();
                    }
                }
                KeyCode::Backspace => {
                    self.action_input.pop();
                }
                KeyCode::Char(value) if is_text_input(key.modifiers) => {
                    self.action_input.push(value)
                }
                _ => {}
            }
            return false;
        }
        if self.input_mode == InputMode::Help {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
                self.input_mode = InputMode::Normal;
            }
            return false;
        }
        if self.input_mode == InputMode::Search {
            match key.code {
                KeyCode::Esc => {
                    self.input.clear();
                    self.input_mode = InputMode::Normal;
                }
                KeyCode::Enter => self.input_mode = InputMode::Normal,
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Char(value) if is_text_input(key.modifiers) => self.input.push(value),
                _ => {}
            }
            self.selected = 0;
            return false;
        }
        if self.input_mode == InputMode::UserFilter {
            match key.code {
                // 对齐 top 的 U：Esc 或空输入回车 = 清除过滤、显示全部。
                KeyCode::Esc => {
                    self.user_filter.clear();
                    self.input_mode = InputMode::Normal;
                }
                KeyCode::Enter => self.input_mode = InputMode::Normal,
                KeyCode::Backspace => {
                    self.user_filter.pop();
                }
                KeyCode::Char(value) if is_text_input(key.modifiers) => {
                    self.user_filter.push(value)
                }
                _ => {}
            }
            self.selected = 0;
            return false;
        }
        if self.input_mode == InputMode::Interval {
            match key.code {
                KeyCode::Esc => {
                    self.interval_input.clear();
                    self.input_mode = InputMode::Normal;
                }
                KeyCode::Enter => {
                    self.input_mode = InputMode::Normal;
                    let raw = std::mem::take(&mut self.interval_input);
                    // 对齐 top：空输入回车 = 保持现值。
                    if !raw.trim().is_empty() {
                        match parse_interval_input(&raw) {
                            Some(millis) => {
                                self.interval_millis = millis.clamp(
                                    bmtop_core::RefreshInterval::MIN_MILLIS,
                                    bmtop_core::RefreshInterval::MAX_MILLIS,
                                )
                            }
                            None => {
                                self.status = self.strings().status_interval_invalid.to_string()
                            }
                        }
                    }
                }
                KeyCode::Backspace => {
                    self.interval_input.pop();
                }
                KeyCode::Char(value) if is_text_input(key.modifiers) => {
                    self.interval_input.push(value)
                }
                _ => {}
            }
            return false;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
            KeyCode::Char('?') => self.input_mode = InputMode::Help,
            KeyCode::Char('/') => {
                self.input.clear();
                self.input_mode = InputMode::Search;
            }
            // 对齐 top：`o` 排序列 / `O`、`R` 升降序 / `s`、`d` 设间隔。
            KeyCode::Char('o') => self.sort_key = self.sort_key.next(),
            KeyCode::Char('O') | KeyCode::Char('R') => self.sort_descending = !self.sort_descending,
            // Linux top 的直达排序键：P=CPU M=内存 N=PID。
            KeyCode::Char('P') => self.sort_key = SortKey::Cpu,
            KeyCode::Char('M') => self.sort_key = SortKey::Memory,
            KeyCode::Char('N') => self.sort_key = SortKey::Pid,
            // 活动监视器的「能耗」标签页心智：E 能耗影响，W 瓦特。
            KeyCode::Char('E') => self.sort_key = SortKey::Energy,
            KeyCode::Char('W') => self.sort_key = SortKey::Power,
            KeyCode::Char('s') | KeyCode::Char('d') => {
                self.interval_input.clear();
                self.input_mode = InputMode::Interval;
            }
            KeyCode::Char('c') => self.show_full_path = !self.show_full_path,
            KeyCode::Char('f') => {
                self.fps_enabled = !self.fps_enabled;
                self.refresh_requested = true; // 立即采一拍，开关反馈不用等下个周期
            }
            KeyCode::Char('i') => self.hide_idle = !self.hide_idle,
            KeyCode::Char('V') => self.tree_view = !self.tree_view,
            KeyCode::Char('H') => self.thread_view = !self.thread_view,
            KeyCode::Char('u') => {
                self.input_mode = InputMode::UserFilter;
            }
            // 运行时调采样间隔：250ms 步进，钳在既有区间内。'=' 是 '+' 的不移位别名。
            KeyCode::Char('+') | KeyCode::Char('=') => self.adjust_interval(250),
            KeyCode::Char('-') => self.adjust_interval(-250),
            KeyCode::Char(' ') => {
                self.paused = !self.paused;
                self.status = self.live_status().to_string();
            }
            KeyCode::Char('r') => {
                self.status = self.strings().status_refreshing.to_string();
                self.refresh_requested = true;
            }
            // `k` 是 Linux top 的杀进程键，与 `x`（TERM）同义；`X` 仍是强杀。
            KeyCode::Char('x') | KeyCode::Char('k') | KeyCode::Char('X') => {
                if let Some((_, process)) = self.filtered_processes().get(self.selected).copied() {
                    let signal = if key.code == KeyCode::Char('X') {
                        ProcessSignalKind::Kill
                    } else {
                        ProcessSignalKind::Terminate
                    };
                    let template = if signal == ProcessSignalKind::Kill {
                        self.strings().confirm_kill
                    } else {
                        self.strings().confirm_terminate
                    };
                    let confirmation = template.replace("{pid}", &process.pid.to_string());
                    self.pending_action = Some(PendingAction {
                        pid: process.pid,
                        start_seconds: process.start_time_seconds,
                        start_microseconds: process.start_time_microseconds,
                        signal,
                        confirmation,
                    });
                    self.action_input.clear();
                    self.input_mode = InputMode::Action;
                }
            }
            // 硬件 / 传感器页：↑↓ 移动左侧分区游标，PgUp/PgDn 滚动右侧详情。
            KeyCode::Up if self.uses_section_list() => {
                self.section_selected = self.section_selected.saturating_sub(1);
                self.detail_scroll = 0;
            }
            KeyCode::Down if self.uses_section_list() => {
                self.section_selected = self
                    .section_selected
                    .saturating_add(1)
                    .min(self.sections.len().saturating_sub(1));
                self.detail_scroll = 0;
            }
            KeyCode::Home | KeyCode::Char('g') if self.uses_section_list() => {
                self.section_selected = 0;
                self.detail_scroll = 0;
            }
            KeyCode::End | KeyCode::Char('G') if self.uses_section_list() => {
                self.section_selected = self.sections.len().saturating_sub(1);
                self.detail_scroll = 0;
            }
            KeyCode::PageUp if self.uses_section_list() => {
                self.detail_scroll = self.detail_scroll.saturating_sub(DETAIL_SCROLL_STEP)
            }
            KeyCode::PageDown if self.uses_section_list() => {
                self.detail_scroll = self.detail_scroll.saturating_add(DETAIL_SCROLL_STEP)
            }
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => {
                self.selected = self
                    .selected
                    .saturating_add(1)
                    .min(self.filtered_processes().len().saturating_sub(1))
            }
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(10),
            KeyCode::PageDown => {
                self.selected = self
                    .selected
                    .saturating_add(10)
                    .min(self.filtered_processes().len().saturating_sub(1))
            }
            KeyCode::Left | KeyCode::BackTab => self.cycle_mode(-1),
            KeyCode::Right | KeyCode::Tab => self.cycle_mode(1),
            KeyCode::Home | KeyCode::Char('g') => self.selected = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.filtered_processes().len().saturating_sub(1)
            }
            KeyCode::Char(value) if ('1'..='9').contains(&value) => {
                self.request_mode(AppMode::from_number(value.to_digit(10).unwrap() as u8))
            }
            KeyCode::F(number) if (1..=9).contains(&number) => {
                self.request_mode(AppMode::from_number(number))
            }
            _ => {}
        }
        false
    }

    pub(crate) fn take_completed_action(&mut self) -> Option<PendingAction> {
        self.completed_action.take()
    }
}

/// 文本输入只排除 Ctrl / Alt / Super。增强键盘协议的终端会给大写字母
/// 带上 SHIFT 修饰键，`modifiers.is_empty()` 会把大写输入整个吃掉。
fn is_text_input(modifiers: KeyModifiers) -> bool {
    !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
}

/// 树状视图：按 PPID 做 DFS，兄弟间保持传入的排序；父不在可见集合
/// （被过滤 / 无权限 / 自指）的行成为根。深度用于渲染缩进。
fn tree_order(rows: Vec<&bmtop_core::ProcessRow>) -> Vec<(usize, &bmtop_core::ProcessRow)> {
    use std::collections::{HashMap, HashSet};
    let visible: HashSet<i32> = rows.iter().map(|row| row.pid).collect();
    let mut children: HashMap<i32, Vec<&bmtop_core::ProcessRow>> = HashMap::new();
    let mut roots = Vec::new();
    for row in &rows {
        if row.parent_pid == row.pid || !visible.contains(&row.parent_pid) {
            roots.push(*row);
        } else {
            children.entry(row.parent_pid).or_default().push(row);
        }
    }
    let mut output = Vec::with_capacity(rows.len());
    let mut emitted = HashSet::new();
    // 栈里逆序压入，弹出顺序即排序顺序。
    let mut stack: Vec<(usize, &bmtop_core::ProcessRow)> =
        roots.into_iter().rev().map(|row| (0, row)).collect();
    while let Some((depth, row)) = stack.pop() {
        if !emitted.insert(row.pid) {
            continue;
        }
        output.push((depth, row));
        if let Some(kids) = children.get(&row.pid) {
            for kid in kids.iter().rev() {
                stack.push((depth + 1, kid));
            }
        }
    }
    output
}

/// `s` 提示符的输入：裸数字按秒（top 语义），`500ms`/`2s` 也接受。
/// 全进程能耗影响求和。没有任何一行拿到读数（首个采样 / rusage 全失败）时
/// 返回 `None`，让走势和卡片显示「无数据」而不是 0。
pub(crate) fn total_energy_impact(snapshot: &SystemSnapshot) -> Option<f64> {
    let mut total = None;
    for impact in snapshot
        .processes
        .iter()
        .filter_map(|row| row.energy_impact)
    {
        *total.get_or_insert(0.0) += impact;
    }
    total
}

pub(crate) fn parse_interval_input(value: &str) -> Option<u64> {
    let trimmed = value.trim();
    if let Some(millis) = trimmed.strip_suffix("ms") {
        return millis.trim().parse::<u64>().ok();
    }
    let seconds = trimmed.strip_suffix('s').unwrap_or(trimmed).trim();
    seconds.parse::<u64>().ok().map(|value| value * 1_000)
}
