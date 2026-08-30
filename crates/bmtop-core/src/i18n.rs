//! 界面文案的中英对照表。
//!
//! 一条文案在 `strings!` 里占一行，中英并排。少写一种语言就编译不过，
//! 所以不会出现某个语言漏翻的半成品。

use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// 声明一条文案：`字段名 => 中文 / English`。
macro_rules! strings {
    ($($(#[$note:meta])* $field:ident => $chinese:literal / $english:literal),* $(,)?) => {
        /// 一整套界面文案。字段按出现位置分组排列。
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct Strings {
            $($(#[$note])* pub $field: &'static str,)*
        }

        const CHINESE_STRINGS: Strings = Strings { $($field: $chinese,)* };
        const ENGLISH_STRINGS: Strings = Strings { $($field: $english,)* };
    };
}

strings! {
    // —— 模式名（同时用于模式条、标题栏、面板标题）——
    mode_overview   => "概览"     / "Overview",
    mode_processes  => "进程"     / "Processes",
    mode_cpu        => "CPU"      / "CPU",
    mode_memory     => "内存"     / "Memory",
    mode_network    => "网络"     / "Network",
    mode_disk       => "磁盘"     / "Disk",
    mode_gpu        => "GPU"      / "GPU",
    mode_hardware   => "硬件"     / "Hardware",
    mode_sensors    => "传感器"   / "Sensors",

    // —— 状态栏与提示 ——
    status_sampling      => "正在采样"       / "Sampling",
    status_paused        => "已暂停"         / "PAUSED",
    status_live          => "LIVE"           / "LIVE",
    status_refreshing    => "请求立即刷新"   / "Refresh requested",
    status_action_cancel => "已取消进程操作" / "Process action cancelled",
    status_action_mismatch => "确认文本不匹配，按 Esc 取消" / "Confirmation does not match, press Esc to cancel",
    status_sample_failed   => "采样失败: {error}"     / "Sampling failed: {error}",
    status_action_failed   => "进程操作失败: {error}" / "Process action failed: {error}",
    detail_load_failed     => "详情加载失败: {error}" / "Detail load failed: {error}",
    hint_normal   => "↑↓ 选择  ←→ 切页  / 搜索  Space 暂停  x 结束  X 强制结束  ? 帮助  q 退出"
                   / "↑↓ select  ←→ switch page  / search  Space pause  x term  X kill  ? help  q quit",
    hint_sections => "↑↓ 选择分区  PgUp/PgDn 滚动详情  1…9 切换模式  ? 帮助  q 退出"
                   / "↑↓ select section  PgUp/PgDn scroll detail  1…9 switch mode  ? help  q quit",
    hint_search   => "Enter 确认  Esc 清除"   / "Enter confirm  Esc clear",
    hint_interval => "秒（如 2）或 500ms · Enter 确认  Esc 取消"
                   / "seconds (e.g. 2) or 500ms · Enter confirm  Esc cancel",
    status_interval_invalid => "无效的间隔，示例：2 或 500ms" / "Invalid interval, e.g. 2 or 500ms",
    hint_action   => "Enter 确认  Esc 取消"   / "Enter confirm  Esc cancel",
    action_fallback => "进程操作"             / "Process action",
    confirm_kill      => "输入 PID {pid} 确认强制结束" / "Type PID {pid} to confirm force kill",
    confirm_terminate => "输入 y 确认结束 PID {pid}"   / "Type y to confirm terminating PID {pid}",

    // —— 通用 ——
    waiting_snapshot => "等待首个快照…" / "Waiting for first snapshot…",
    loading          => "读取中…"       / "Loading…",
    detail           => "详情"          / "Details",
    unavailable      => "不可用"        / "unavailable",

    // —— 概览 ——
    panel_system_load => "系统负载" / "System Load",
    title_uptime      => "运行 {uptime}" / "up {uptime}",
    load_prefix       => "负载"     / "Load",
    summary_cpu       => "用户 {user} · 系统 {system}"      / "user {user} · sys {system}",
    summary_memory    => "{used} / {total} · 压力 {pressure}" / "{used} / {total} · pressure {pressure}",
    summary_gpu       => "空闲 {idle}"                       / "idle {idle}",

    // —— CPU 页 ——
    cpu_total  => "总计" / "Total",
    cpu_user   => "用户" / "User",
    cpu_system => "系统" / "System",
    cpu_idle   => "空闲" / "Idle",

    // —— 内存页 ——
    memory_used       => "已用"   / "Used",
    memory_available  => "可用"   / "Available",
    memory_pressure   => "压力"   / "Pressure",
    memory_wired      => "Wired"  / "Wired",
    memory_compressed => "压缩"   / "Compressed",
    memory_active     => "活跃"   / "Active",
    memory_inactive   => "非活跃" / "Inactive",
    memory_free       => "空闲"   / "Free",
    memory_purgeable  => "可清除" / "Purgeable",
    memory_swap       => "Swap"   / "Swap",
    memory_swap_value => "换入 {in} · 换出 {out}" / "in {in} · out {out}",
    memory_swap_bytes => "{used} / {total} · 换入 {in} · 换出 {out}" / "{used} / {total} · in {in} · out {out}",

    // —— 磁盘页 ——
    disk_usage => "{used} 已用 / {total}" / "{used} used / {total}",

    // —— 网络页 ——
    network_down       => "↓ 下行"   / "↓ Down",
    network_up         => "↑ 上行"   / "↑ Up",
    network_interface  => "接口"     / "Interface",
    network_rx         => "下行"     / "Down",
    network_tx         => "上行"     / "Up",
    network_total_rx   => "累计下行" / "Total Rx",
    network_total_tx   => "累计上行" / "Total Tx",

    // —— GPU 页 ——
    gpu_unavailable => "GPU 不可用" / "GPU unavailable",
    gpu_name        => "型号"       / "Model",
    gpu_utilization => "使用率"     / "Usage",
    gpu_idle        => "空闲"       / "Idle",
    gpu_trend       => "趋势"       / "Trend",

    // —— SoC（集群 / 功耗 / 温度 / 风扇 / 传感器）——
    cpu_cluster_e => "E 集群" / "E-Cluster",
    cpu_cluster_p => "P 集群" / "P-Cluster",
    cpu_cluster_s => "S 集群" / "S-Cluster",
    cpu_per_core  => "每核心" / "Per core",
    cpu_breakdown => "用户 {user} · 系统 {system} · 空闲 {idle}"
                   / "user {user} · sys {system} · idle {idle}",
    label_power => "功耗" / "Power",
    label_energy => "能耗" / "Energy",
    label_temp  => "温度" / "Temp",
    label_freq  => "频率" / "Freq",
    power_summary => "CPU {cpu} · GPU {gpu} · ANE {ane} · 共 {total}"
                   / "CPU {cpu} · GPU {gpu} · ANE {ane} · total {total}",
    cpu_power_temp => "CPU {watts} · 温度 {temp}" / "CPU {watts} · temp {temp}",
    thermal_pressure => "热压力" / "Thermal pressure",
    thermal_level_0 => "正常" / "Nominal",
    thermal_level_1 => "偏暖" / "Fair",
    thermal_level_2 => "严重" / "Serious",
    thermal_level_3 => "临界" / "Critical",
    thermal_level_4 => "休眠" / "Sleeping",
    sensors_fans => "风扇" / "Fans",
    sensors_unavailable => "SoC 传感器不可用（Intel 或 IOReport 初始化失败）"
                         / "SoC sensors unavailable (Intel or IOReport init failed)",
    fan_rpm => "{rpm} / {max} RPM" / "{rpm} / {max} RPM",
    fan_target_range => "目标 {target} · 范围 {min}–{max}" / "target {target} · range {min}–{max}",
    card_power_total => "共 {total}" / "total {total}",
    sensor_range => "{avg} ({min}–{max})" / "{avg} ({min}–{max})",
    sensor_group_cpu     => "CPU"  / "CPU",
    sensor_group_cpu_e   => "CPU E 核" / "CPU E-Core",
    sensor_group_cpu_p   => "CPU P 核" / "CPU P-Core",
    sensor_group_cpu_die => "CPU Die"  / "CPU Die",
    sensor_group_gpu     => "GPU"  / "GPU",
    sensor_group_soc     => "SoC"  / "SoC",
    sensor_group_memory  => "内存" / "Memory",
    sensor_group_ssd     => "SSD"  / "SSD",
    sensor_group_ambient => "环境" / "Ambient",
    sensor_group_board   => "主板" / "Board",
    sensor_group_vrm     => "VRM"  / "VRM",
    sensor_group_display => "显示" / "Display",
    sensor_group_wireless => "无线" / "Wireless",
    sensor_group_other   => "其他" / "Other",
    disk_volume_count => "{count} 卷" / "{count} volumes",
    disk_io_value => "读 {read} · 写 {write}" / "R {read} · W {write}",
    memory_bandwidth => "带宽" / "BW",
    memory_bandwidth_value => "R {read} · W {write} GB/s" / "R {read} · W {write} GB/s",
    label_battery => "电池" / "Battery",
    battery_charging   => "充电中"   / "charging",
    battery_ac         => "外接电源" / "AC power",
    battery_on_battery => "电池供电" / "on battery",
    label_system_power => "系统" / "System",
    gpu_peak => "峰值" / "Peak",
    gpu_peak_value => "{ghz} GHz · {tflops} TFLOPS" / "{ghz} GHz · {tflops} TFLOPS",
    label_fps => "FPS" / "FPS",
    fps_value => "{fps} FPS · {interval}ms" / "{fps} FPS · {interval}ms",
    fps_permission_hint => "FPS 需要屏幕录制权限（系统设置 → 隐私与安全性）"
                         / "FPS needs Screen Recording permission (System Settings → Privacy)",
    fps_off_hint => "按 f 开启屏幕 FPS" / "press f to enable display FPS",
    section_thunderbolt => "雷雳" / "Thunderbolt",
    section_rdma => "RDMA" / "RDMA",
    column_gpu => "GPU%" / "GPU%",
    field_gpu => "GPU" / "GPU",
    field_virtual => "虚拟内存" / "Virt",
    field_cpu_time => "CPU 时间" / "CPU time",
    help_fps => "屏幕 FPS 开关" / "Toggle display FPS",
    network_peak => "峰值 ↓ {down} ↑ {up}" / "peak ↓ {down} ↑ {up}",

    // —— 进程表与详情 ——
    process_sorted_by     => "进程 · {key} 排序" / "Processes · by {key}",
    process_user_filter   => " · 用户 {user}"     / " · user {user}",
    process_active_only   => " · 仅活跃"           / " · active only",
    process_count         => "{items} 项 · {threads} 线程" / "{items} procs · {threads} threads",
    column_energy  => "能耗" / "NRG",
    column_power   => "功耗" / "PWR",
    column_memory  => "内存" / "MEM",
    column_threads => "线程" / "THR",
    column_user    => "用户" / "USER",
    column_command => "命令" / "COMMAND",
    field_state   => "状态"   / "State",
    field_started => "启动"   / "Started",
    field_memory  => "内存"   / "Memory",
    field_threads => "线程"   / "Threads",
    field_files   => "文件"   / "Files",
    field_disk_read  => "磁盘读" / "Disk R",
    field_disk_write => "磁盘写" / "Disk W",
    field_user    => "用户"   / "User",
    field_parent  => "父进程" / "Parent",
    field_path    => "路径"   / "Path",
    field_arguments => "参数" / "Args",
    field_mode         => "模式" / "Mode",
    field_snapshot     => "快照" / "Snapshot",
    field_capabilities => "能力" / "Capabilities",
    section_count => "{count} 项" / "{count} items",

    // —— 帮助层 ——
    help_title  => "快捷键"       / "Shortcuts",
    help_close  => "Esc / ? 关闭" / "Esc / ? to close",
    help_modes     => "切换模式"             / "Switch mode",
    help_enhanced  => "增强键盘协议终端"     / "Enhanced keyboard protocol",
    help_move      => "移动选择"             / "Move selection",
    help_focus     => "上一页 / 下一页"      / "Previous / next page",
    help_page      => "整页滚动 / 滚动详情"  / "Page scroll / scroll detail",
    help_ends      => "跳到首尾"             / "Jump to first / last",
    help_search    => "搜索过滤"             / "Search filter",
    help_sort      => "排序列 CPU/内存/PID"  / "Sort column CPU/MEM/PID",
    help_sort_energy => "按能耗 / 功耗排序"   / "Sort by energy / power",
    help_sort_direction => "反转升降序"       / "Reverse sort order",
    help_user_filter    => "按用户过滤"       / "Filter by user",
    help_set_interval   => "设置采样间隔"     / "Set sampling interval",
    help_interval       => "间隔 ±250ms"      / "Interval ±250ms",
    help_full_path      => "显示完整路径"     / "Show full path",
    help_hide_idle      => "隐藏空闲进程"     / "Hide idle processes",
    help_tree           => "树状视图"         / "Tree view",
    help_threads        => "线程视图（选中进程）" / "Thread view (selected)",
    help_redraw         => "强制重绘"         / "Force redraw",
    help_refresh   => "立即刷新"             / "Refresh now",
    help_pause     => "暂停 / 继续"          / "Pause / resume",
    help_signal    => "结束 / 强制结束进程"  / "Terminate / force kill",
    help_quit      => "退出并恢复终端"       / "Quit and restore terminal",

    // —— CLI ——
    cli_about       => "macOS 资源与硬件只读监控工具" / "Read-only macOS resource and hardware monitor",
    // 这两条长文案必须写成物理上的一行：rustfmt 会把 `\` 续行拼掉，
    // 续行前的缩进会被烤进字符串里，帮助里就会多出一片空格。
    cli_long_about => "macOS 资源与硬件只读监控工具。\n\n不带子命令时进入交互式面板。每一个面板同时也是一个子命令，输出 json / jsonl / csv，契约版本固定，可直接喂给脚本。"
                    / "Read-only macOS resource and hardware monitor.\n\nRun without a subcommand for the interactive dashboard. Every panel is also a subcommand with stable json / jsonl / csv output, so the same data is usable from scripts.",

    // —— 子命令说明（clap 帮助里每行一句）——
    cli_about_top      => "打开交互式面板（不带子命令时的默认行为）"
                        / "Launch the interactive dashboard (the default when no subcommand is given)",
    cli_about_ps       => "列出进程的 CPU、GPU、内存、能耗影响与估算功耗"
                        / "List processes with CPU, GPU, memory, energy impact and estimated power",
    cli_about_cpu      => "CPU 负载、每核占用，以及 Apple Silicon 的分簇频率与功耗"
                        / "CPU load, per-core usage and (Apple Silicon) per-cluster frequency and power",
    cli_about_memory   => "内存占用、交换空间与内存压力"
                        / "Memory usage, swap and memory pressure",
    cli_about_network  => "各网络接口吞吐，附链路类型（以太网速率 / Wi-Fi 代次）"
                        / "Per-interface throughput plus link type (Ethernet speed / Wi-Fi generation)",
    cli_about_disk     => "各卷容量与磁盘读写吞吐"
                        / "Volume capacity and disk I/O throughput",
    cli_about_gpu      => "GPU 占用、频率、功耗与每进程 GPU 时间"
                        / "GPU utilisation, frequency, power and per-process GPU time",
    cli_about_sensors  => "温度、风扇、热压力与电池"
                        / "Temperatures, fans, thermal pressure and battery",
    cli_about_hardware => "来自 system_profiler 的静态硬件清单"
                        / "Static hardware inventory from system_profiler",
    cli_about_doctor   => "逐项检查本机哪些数据源可读，不可读的说明原因"
                        / "Check which data sources are readable on this machine, and why any are not",
    cli_about_completion => "把 bmtop 的 shell 补全脚本打印到标准输出"
                          / "Print a shell completion script for bmtop to stdout",
    // 同上，单行；三条安装命令靠 \n 各自占一行。
    cli_completion_long => "把 bmtop 的 shell 补全脚本打印到标准输出。\n\n它只负责输出脚本，不安装任何东西、不改任何文件。把输出重定向到你的 shell 启动时会读取的目录，然后开一个新 shell：\n\n  bash  bmtop completion bash > $(brew --prefix)/etc/bash_completion.d/bmtop\n  zsh   bmtop completion zsh  > $(brew --prefix)/share/zsh/site-functions/_bmtop\n  fish  bmtop completion fish > ~/.config/fish/completions/bmtop.fish\n\n没装 Homebrew 的话，bash 的 $BASH_COMPLETION_COMPAT_DIR 或 zsh 的 $fpath 里任意一个目录都行，`echo $fpath` 可以看到候选。\n\n装好之后按 <Tab> 能补全子命令、参数以及参数的取值——比如 `bmtop ps --sort <Tab>` 会列出 cpu / memory / pid / energy / power。"
                         / "Print a shell completion script for bmtop to stdout.\n\nIt only writes the script to stdout — nothing is installed and no file is touched. Redirect it into the directory your shell loads at startup, then open a new shell:\n\n  bash  bmtop completion bash > $(brew --prefix)/etc/bash_completion.d/bmtop\n  zsh   bmtop completion zsh  > $(brew --prefix)/share/zsh/site-functions/_bmtop\n  fish  bmtop completion fish > ~/.config/fish/completions/bmtop.fish\n\nWithout Homebrew, any directory on bash's $BASH_COMPLETION_COMPAT_DIR or zsh's $fpath works; `echo $fpath` shows the candidates.\n\nOnce loaded, <Tab> completes subcommands, flags and flag values — for instance `bmtop ps --sort <Tab>` offers cpu / memory / pid / energy / power.",

    // —— 全局与子命令参数说明 ——
    cli_help_interval => "采样间隔，例如 `2s`、`500ms`" / "Sampling interval, e.g. `2s`, `500ms`",
    cli_help_format   => "一次性子命令的输出格式" / "Output format for one-shot subcommands",
    cli_help_watch    => "持续采样直到被中断，而不是只打印一次"
                       / "Keep sampling until interrupted instead of printing once",
    cli_help_count    => "采样 N 次后退出（隐含 --watch 的循环语义），供脚本使用"
                       / "Sample N times then exit (implies the --watch loop); for scripting",
    cli_help_enhanced => "合并一次 sudo powermetrics 采样（只有 gpu 和 sensors 接受）"
                       / "Merge one sudo `powermetrics` sample in (only `gpu` and `sensors` accept it)",
    cli_help_sensitive => "不脱敏硬件标识（序列号、UUID）"
                        / "Do not redact hardware identifiers (serial numbers, UUIDs)",
    cli_help_ps_pid   => "只看这一个 PID" / "Only this PID",
    cli_help_ps_user  => "只看该用户拥有的进程" / "Only processes owned by this user",
    cli_help_ps_sort  => "排序列：cpu | memory | pid | energy | power"
                       / "Sort column: cpu | memory | pid | energy | power",
    cli_help_ps_limit => "排序后只保留前 N 行" / "Keep only the first N rows (truncated after sorting)",
    cli_help_top_mode => "直接打开某一页：1 概览、2 进程 …… 9 传感器"
                       / "Open on a specific page: 1 overview, 2 processes, … 9 sensors",
    cli_help_net_conn => "同时列出活动的网络连接" / "Also list active network connections",
    cli_help_hw_cat   => "只看某一类，例如 `SPDisplaysDataType`；省略则全部列出"
                       / "Limit to one category, e.g. `SPDisplaysDataType`; omit to list all",
    cli_help_shell    => "为哪个 shell 生成脚本" / "Which shell to emit the script for",
    cli_lang_help   => "界面语言（zh 或 en），默认跟随 LC_ALL / LC_MESSAGES / LANG"
                     / "Interface language (zh or en); defaults to LC_ALL / LC_MESSAGES / LANG",
    cli_signal_sent => "已发送 {signal} 到 PID {pid}" / "Sent {signal} to PID {pid}",
    cli_hardware_categories => "硬件分类" / "Hardware categories",
    cli_keyboard_note => "Command+1…9 仅在终端报告增强键盘协议时可用"
                       / "Command+1…9 works only when the terminal reports an enhanced keyboard protocol",
}

/// 界面语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Chinese,
    #[default]
    English,
}

impl Language {
    pub const fn strings(self) -> &'static Strings {
        match self {
            Self::Chinese => &CHINESE_STRINGS,
            Self::English => &ENGLISH_STRINGS,
        }
    }

    pub const fn code(self) -> &'static str {
        match self {
            Self::Chinese => "zh",
            Self::English => "en",
        }
    }

    /// 按 POSIX 优先级读 `LC_ALL` → `LC_MESSAGES` → `LANG`。
    /// 只有语言标签以 `zh` 开头才是中文；未设置或 `C` / `POSIX` 都按英文。
    pub fn from_environment() -> Self {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|value| !value.is_empty())
            .map(|value| Self::from_locale(&value))
            .unwrap_or_default()
    }

    /// `zh_CN.UTF-8` / `zh-Hant` / `zh` → 中文，其余一律英文。
    pub fn from_locale(locale: &str) -> Self {
        let tag = locale
            .split(['.', '@'])
            .next()
            .unwrap_or(locale)
            .to_ascii_lowercase();
        if tag == "zh" || tag.starts_with("zh_") || tag.starts_with("zh-") {
            Self::Chinese
        } else {
            Self::English
        }
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[error("language must be zh or en")]
pub struct LanguageParseError;

impl FromStr for Language {
    type Err = LanguageParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "zh" | "zh-cn" | "zh_cn" | "chinese" => Ok(Self::Chinese),
            "en" | "en-us" | "en_us" | "english" => Ok(Self::English),
            _ => Err(LanguageParseError),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_detection_only_treats_zh_as_chinese() {
        assert_eq!(Language::from_locale("zh_CN.UTF-8"), Language::Chinese);
        assert_eq!(Language::from_locale("zh"), Language::Chinese);
        assert_eq!(Language::from_locale("zh-Hant"), Language::Chinese);
        assert_eq!(Language::from_locale("en_US.UTF-8"), Language::English);
        assert_eq!(Language::from_locale("C"), Language::English);
        assert_eq!(Language::from_locale("POSIX"), Language::English);
        assert_eq!(Language::from_locale(""), Language::English);
        // 前缀相同但不是中文的语言标签不能误判。
        assert_eq!(Language::from_locale("zhs_XX"), Language::English);
    }

    #[test]
    fn explicit_language_flag_accepts_both_spellings() {
        assert_eq!("zh".parse::<Language>().unwrap(), Language::Chinese);
        assert_eq!("EN".parse::<Language>().unwrap(), Language::English);
        assert_eq!("zh_CN".parse::<Language>().unwrap(), Language::Chinese);
        assert!("fr".parse::<Language>().is_err());
        assert!("".parse::<Language>().is_err());
    }

    #[test]
    fn every_string_is_translated_differently_where_it_should_be() {
        let zh = Language::Chinese.strings();
        let en = Language::English.strings();
        // 少数条目两种语言天然相同（CPU / GPU / Swap / LIVE / Wired）。
        assert_eq!(zh.mode_cpu, en.mode_cpu);
        assert_ne!(zh.mode_overview, en.mode_overview);
        assert_ne!(zh.help_quit, en.help_quit);
        assert_ne!(zh.cli_about, en.cli_about);
    }
}
