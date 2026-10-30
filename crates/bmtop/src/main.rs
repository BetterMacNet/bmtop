use anyhow::Result;
use bmtop_core::{
    AppMode, CapabilityState, JsonEnvelope, Language, PrivacyMode, RefreshInterval, SystemSnapshot,
    SCHEMA_VERSION,
};
use bmtop_macos::{
    disk_report, hardware_report_with_privacy, network_connections, sample_powermetrics,
    send_signal_if_identity, sensor_report_with_privacy, uptime_seconds, CollectorConfig,
    CollectorError, MacCollector, ProcessSignal,
};
use bmtop_tui::{
    keyboard_enhancement_supported, run_with_options as run_tui_with_options, ModeDetail,
    ProcessSignalKind,
};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, shells};
mod config;
mod output;
mod sections;

use sections::hardware_sections;
use serde_json::json;
use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::time::Duration;

/// 类型化的 CLI 错误。以前 `exit_code` 靠错误文本子串匹配分类，
/// 一改措辞或翻译就悄悄退化成 70。
#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error("capability-unavailable: {0}")]
    CapabilityUnavailable(String),
}

fn usage(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(CliError::Usage(message.into()))
}

#[derive(Debug, Parser)]
// 所有面向用户的帮助文案都在 i18n.rs，由 execute() 在运行时按语言注入；
// 这里不写死英文，否则 `--lang zh --help` 会中英各一半。
#[command(name = "bmtop", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(long, global = true, env = "BMTOP_LANG", value_name = "zh|en")]
    lang: Option<Language>,
    #[arg(
        short = 'i',
        long,
        global = true,
        env = "BMTOP_INTERVAL",
        value_name = "DURATION"
    )]
    interval: Option<String>,
    #[arg(long, global = true, default_value_t = OutputFormat::Table)]
    format: OutputFormat,
    #[arg(long, global = true)]
    watch: bool,
    #[arg(short = 'n', long, global = true, value_name = "N")]
    count: Option<u32>,
    #[arg(long, global = true)]
    enhanced: bool,
    #[arg(long, global = true, conflicts_with = "privacy")]
    show_sensitive: bool,
    #[arg(
        long,
        global = true,
        env = "BMTOP_PRIVACY",
        value_name = "strict|default|off"
    )]
    privacy: Option<PrivacyMode>,
    #[arg(long, global = true, value_name = "PATH", conflicts_with = "no_config")]
    config: Option<std::path::PathBuf>,
    #[arg(long, global = true)]
    no_config: bool,
    #[arg(long, global = true)]
    print_config: bool,
    #[arg(long, global = true)]
    schema_version: bool,
    #[arg(long, global = true)]
    quick: bool,
    #[arg(long, global = true, value_name = "PATH")]
    diagnostics_file: Option<std::path::PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Top {
        #[arg(long)]
        mode: Option<u8>,
    },
    Ps {
        #[arg(long)]
        pid: Option<i32>,
        #[arg(long)]
        user: Option<String>,
        #[arg(long, default_value = "cpu", value_parser = ["cpu", "gpu", "memory", "mem", "pid", "energy", "nrg", "power", "watts"])]
        sort: String,
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
    Cpu,
    Memory,
    Network {
        #[arg(long)]
        connections: bool,
    },
    Disk,
    Gpu,
    Sensors,
    Hardware {
        category: Option<String>,
    },
    Doctor,
    Completion {
        shell: Shell,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    Table,
    Json,
    Jsonl,
    Csv,
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Table => "table",
            Self::Json => "json",
            Self::Jsonl => "jsonl",
            Self::Csv => "csv",
        })
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Shell {
    Bash,
    Fish,
    Zsh,
}

fn main() {
    if let Err(error) = execute() {
        eprintln!("bmtop: {error:#}");
        std::process::exit(exit_code(&error));
    }
}

/// `--help` 在 clap 解析期间就打印了，所以帮助文案的语言必须在解析之前定下来。
/// 先扫一遍 argv 找 `--lang`，找不到再退回环境变量。
fn preferred_language<I: IntoIterator<Item = String>>(arguments: I) -> Language {
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if let Some(value) = argument.strip_prefix("--lang=") {
            if let Ok(language) = value.parse() {
                return language;
            }
        } else if argument == "--lang" {
            if let Some(Ok(language)) = arguments.next().map(|value| value.parse()) {
                return language;
            }
        }
    }
    std::env::var("BMTOP_LANG")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(Language::from_environment)
}

/// 把 i18n 里的帮助文案挂到 clap 上。
///
/// 文案不能写成 `#[command(about = "…")]` 那样的属性：那是编译期常量，
/// 而 bmtop 的帮助要跟着 `--lang` / LANG 走（见 cli_contracts 的两个语言用例）。
fn localized_help(command: clap::Command, text: &'static bmtop_core::Strings) -> clap::Command {
    let mut command = command
        .about(text.cli_about)
        .long_about(text.cli_long_about)
        .mut_arg("lang", |argument| argument.help(text.cli_lang_help))
        .mut_arg("interval", |argument| argument.help(text.cli_help_interval))
        .mut_arg("format", |argument| argument.help(text.cli_help_format))
        .mut_arg("watch", |argument| argument.help(text.cli_help_watch))
        .mut_arg("count", |argument| argument.help(text.cli_help_count))
        .mut_arg("enhanced", |argument| argument.help(text.cli_help_enhanced))
        .mut_arg("show_sensitive", |argument| {
            argument.help(text.cli_help_sensitive)
        });
    for (name, about) in [
        ("top", text.cli_about_top),
        ("ps", text.cli_about_ps),
        ("cpu", text.cli_about_cpu),
        ("memory", text.cli_about_memory),
        ("network", text.cli_about_network),
        ("disk", text.cli_about_disk),
        ("gpu", text.cli_about_gpu),
        ("sensors", text.cli_about_sensors),
        ("hardware", text.cli_about_hardware),
        ("doctor", text.cli_about_doctor),
        ("completion", text.cli_about_completion),
    ] {
        command = command.mut_subcommand(name, |sub| sub.about(about));
    }
    let chinese = text.mode_overview == "概览";
    for (name, zh, en) in [
        (
            "privacy",
            "隐私策略：strict / default / off",
            "Privacy policy: strict / default / off",
        ),
        (
            "config",
            "读取指定 TOML 配置",
            "Read a specific TOML configuration",
        ),
        (
            "no_config",
            "忽略所有配置文件",
            "Ignore all configuration files",
        ),
        (
            "print_config",
            "输出合并后的配置后退出",
            "Print resolved preferences and exit",
        ),
        (
            "schema_version",
            "输出机器接口版本后退出",
            "Print machine schema version and exit",
        ),
        (
            "quick",
            "单次 CPU/GPU/传感器使用 250ms 采样窗口",
            "Use a 250ms one-shot CPU/GPU/sensors window",
        ),
        (
            "diagnostics_file",
            "TUI 退出时写入不含快照的性能诊断文件",
            "Write timing-only diagnostics when the TUI exits",
        ),
    ] {
        command = command.mut_arg(name, |arg| arg.help(if chinese { zh } else { en }));
    }
    // 各子命令自己的参数说明。
    command
        .mut_subcommand("top", |sub| {
            sub.mut_arg("mode", |argument| argument.help(text.cli_help_top_mode))
        })
        .mut_subcommand("ps", |sub| {
            sub.mut_arg("pid", |argument| argument.help(text.cli_help_ps_pid))
                .mut_arg("user", |argument| argument.help(text.cli_help_ps_user))
                .mut_arg("sort", |argument| argument.help(text.cli_help_ps_sort))
                .mut_arg("limit", |argument| argument.help(text.cli_help_ps_limit))
        })
        .mut_subcommand("network", |sub| {
            sub.mut_arg("connections", |argument| {
                argument.help(text.cli_help_net_conn)
            })
        })
        .mut_subcommand("hardware", |sub| {
            sub.mut_arg("category", |argument| argument.help(text.cli_help_hw_cat))
        })
        .mut_subcommand("completion", |sub| {
            sub.long_about(text.cli_completion_long)
                .mut_arg("shell", |argument| argument.help(text.cli_help_shell))
        })
}

fn execute() -> Result<()> {
    let language = preferred_language(std::env::args().skip(1));
    let command = localized_help(Cli::command(), language.strings());
    let cli = Cli::from_arg_matches(&command.get_matches())?;
    if cli.schema_version {
        println!("{SCHEMA_VERSION}");
        return Ok(());
    }
    if let Some(Command::Completion { shell }) = &cli.command {
        return completion(*shell, cli.lang.unwrap_or(language));
    }
    let mut preferences = config::load(cli.config.as_deref(), cli.no_config)
        .map_err(|error| usage(format!("{error:#}")))?;
    if let Some(value) = &cli.interval {
        preferences.interval = value.clone();
    }
    if let Some(value) = cli.lang {
        preferences.language = Some(value.code().into());
    }
    if let Some(value) = cli.privacy {
        preferences.privacy = value;
    }
    if cli.show_sensitive {
        preferences.privacy = PrivacyMode::Off;
    }
    if let Some(Command::Top { mode: Some(mode) }) = &cli.command {
        preferences.default_mode =
            AppMode::from_number(*mode).ok_or_else(|| usage("mode must be between 1 and 9"))?;
    }
    let interval = parse_interval(&preferences.interval)?;
    let language = preferences.language()?;
    preferences.language = Some(language.code().into());
    if cli.print_config {
        println!("{}", toml::to_string_pretty(&preferences)?);
        return Ok(());
    }
    if cli.enhanced && !matches!(cli.command, Some(Command::Gpu) | Some(Command::Sensors)) {
        return Err(usage(
            "--enhanced only applies to `bmtop gpu` and `bmtop sensors`",
        ));
    }
    if cli.enhanced && cli.format == OutputFormat::Csv {
        return Err(usage("enhanced output requires table, JSON or JSONL"));
    }
    let samples = sample_count(cli.watch, cli.count)?;
    if cli.quick
        && (!matches!(
            cli.command,
            Some(Command::Cpu | Command::Gpu | Command::Sensors)
        ) || samples != 1
            || cli.watch)
    {
        return Err(usage("--quick requires a one-shot cpu/gpu/sensors command"));
    }
    let is_top = matches!(cli.command, None | Some(Command::Top { .. }));
    if is_top && (cli.format != OutputFormat::Table || cli.watch || cli.count.is_some()) {
        return Err(usage(
            "interactive top does not accept --format/--watch/--count",
        ));
    }
    if !is_top && cli.diagnostics_file.is_some() {
        return Err(usage(
            "--diagnostics-file currently applies only to interactive top",
        ));
    }
    if matches!(cli.command, Some(Command::Doctor)) && (cli.watch || cli.count.is_some()) {
        return Err(usage(
            "doctor is a one-shot diagnostic; --watch/--count are unsupported",
        ));
    }
    if cli.format == OutputFormat::Json && samples > 1 {
        return Err(usage(
            "--watch/--count requires --format jsonl, csv or table",
        ));
    }
    if cli.format == OutputFormat::Csv
        && matches!(
            cli.command,
            Some(Command::Hardware { .. } | Command::Sensors | Command::Doctor)
        )
    {
        return Err(usage(
            "CSV is not supported for hardware, sensors or doctor; use JSON/JSONL",
        ));
    }
    if preferences.privacy == PrivacyMode::Off && io::stderr().is_terminal() {
        eprintln!(
            "{}",
            if language == Language::Chinese {
                "警告：隐私过滤已关闭，输出可能包含序列号、标识符和命令行密钥。"
            } else {
                "Warning: privacy filtering is OFF; output may contain identifiers and command-line secrets."
            }
        );
    }
    match cli.command {
        Some(Command::Hardware { category }) => {
            for round in 0..samples {
                if round > 0 {
                    std::thread::sleep(Duration::from_millis(interval.as_millis()));
                }
                run_hardware(
                    category.as_deref(),
                    preferences.privacy,
                    cli.format,
                    language,
                )?;
            }
            Ok(())
        }
        Some(Command::Doctor) => run_doctor(cli.format, language),
        Some(Command::Top { .. }) | None => {
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                return Err(usage(
                    "interactive mode requires a TTY; use `bmtop ps --format json` for pipelines",
                ));
            }
            if cfg!(not(target_os = "macos")) {
                return Err(CollectorError::UnsupportedPlatform.into());
            }
            let diagnostic = cli
                .diagnostics_file
                .as_deref()
                .map(config::create_private)
                .transpose()?;
            run_top(interval, &preferences, diagnostic)
        }
        Some(command) => run_metric(
            command,
            MetricOptions {
                interval,
                format: cli.format,
                samples,
                privacy: preferences.privacy,
                enhanced: cli.enhanced,
                quick: cli.quick,
                language,
            },
        ),
    }
}

fn sample_count(watch: bool, count: Option<u32>) -> Result<u32> {
    match count {
        Some(0) => Err(usage("--count must be at least 1")),
        Some(count) => Ok(count),
        None if watch => Ok(u32::MAX),
        None => Ok(1),
    }
}

fn run_top(
    interval: RefreshInterval,
    preferences: &config::Config,
    diagnostic: Option<std::fs::File>,
) -> Result<()> {
    let initial = preferences.default_mode;
    let language = preferences.language()?;
    let privacy = preferences.privacy;
    let options = preferences.tui_options()?;
    let mut collector = MacCollector::new(CollectorConfig {
        show_sensitive: privacy == PrivacyMode::Off,
        ..CollectorConfig::default()
    });
    let report = run_tui_with_options(
        interval,
        initial,
        language,
        options,
        move |detail_pid, fps_enabled| {
            collector.set_fps_enabled(fps_enabled);
            let mut snapshot = collector
                .snapshot(detail_pid)
                .map_err(|error| error.to_string())?;
            privacy.redact_snapshot(&mut snapshot);
            Ok(snapshot)
        },
        move |mode| match mode {
            // 概览页只用根卷，但拉取成本和全量一样，就复用同一份结果。
            AppMode::Overview | AppMode::Disk => disk_report()
                .map(|mut disks| {
                    privacy.redact_disks(&mut disks);
                    ModeDetail::Disks(disks)
                })
                .map_err(|error| error.to_string()),
            AppMode::Hardware => hardware_report_with_privacy(privacy)
                .map(|report| {
                    let mut sections = hardware_sections(&report.sections, language);
                    sections.extend(transport_sections(language, privacy));
                    ModeDetail::Sections(sections)
                })
                .map_err(|error| error.to_string()),
            // 传感器页改为随快照实时渲染（SoC 数据），不再需要懒加载分区。
            _ => Ok(ModeDetail::None),
        },
        |signal, pid, start_seconds, start_microseconds| {
            let signal = match signal {
                ProcessSignalKind::Terminate => ProcessSignal::Terminate,
                ProcessSignalKind::Kill => ProcessSignal::Kill,
            };
            send_signal_if_identity(pid, start_seconds, start_microseconds, signal)
                .map(|_| {
                    language
                        .strings()
                        .cli_signal_sent
                        .replace("{signal}", signal_name(signal))
                        .replace("{pid}", &pid.to_string())
                })
                .map_err(|error| error.to_string())
        },
    )?;
    if let Some(file) = diagnostic {
        config::write_diagnostics(file, &report)?;
    }
    Ok(())
}

struct MetricOptions {
    interval: RefreshInterval,
    format: OutputFormat,
    samples: u32,
    privacy: PrivacyMode,
    enhanced: bool,
    quick: bool,
    language: Language,
}

fn run_metric(command: Command, options: MetricOptions) -> Result<()> {
    let MetricOptions {
        interval,
        format,
        samples,
        privacy,
        enhanced,
        quick,
        language,
    } = options;
    let kind = match &command {
        Command::Ps { .. } => "processes",
        Command::Cpu => "cpu",
        Command::Memory => "memory",
        Command::Network { connections: true } => "network_connections",
        Command::Network { connections: false } => "network",
        Command::Disk => "disk",
        Command::Gpu => "gpu",
        Command::Sensors => "sensors",
        _ => unreachable!(),
    };
    let window = if quick {
        Duration::from_millis(250)
    } else {
        soc_window(interval)
    };
    if matches!(
        command,
        Command::Disk | Command::Sensors | Command::Network { connections: true }
    ) {
        for round in 0..samples {
            if round > 0 {
                std::thread::sleep(Duration::from_millis(interval.as_millis()));
            }
            let data = match command {
                Command::Disk => {
                    let mut disks = disk_report().map_err(anyhow::Error::new)?;
                    privacy.redact_disks(&mut disks);
                    serde_json::to_value(disks)?
                }
                Command::Sensors => {
                    let sensors =
                        sensor_report_with_privacy(privacy).map_err(anyhow::Error::new)?;
                    let soc = bmtop_macos::soc::sample_soc_once(window);
                    if enhanced {
                        let power = sample_powermetrics(interval).map_err(anyhow::Error::new)?;
                        json!({ "sensors": sensors, "soc": soc, "powermetrics": power })
                    } else {
                        json!({ "sensors": sensors, "soc": soc })
                    }
                }
                _ => {
                    let mut rows = network_connections().map_err(anyhow::Error::new)?;
                    if privacy == PrivacyMode::Strict {
                        for row in &mut rows {
                            row.command = None;
                            row.endpoint = None;
                        }
                    }
                    json!({ "connections": rows })
                }
            };
            if format == OutputFormat::Csv {
                print!("{}", output::report_csv(kind, &data, round == 0)?);
            } else {
                print_report(kind, data, format)?;
            }
        }
        return Ok(());
    }
    let mut collector = MacCollector::new(CollectorConfig {
        show_sensitive: privacy == PrivacyMode::Off,
        ..CollectorConfig::default()
    });
    let detail_pid = match &command {
        Command::Ps { pid, .. } => *pid,
        _ => None,
    };
    if samples == 1 && matches!(kind, "cpu" | "gpu") {
        collector.snapshot(detail_pid).map_err(anyhow::Error::new)?;
        if io::stderr().is_terminal() {
            if language == Language::Chinese {
                eprintln!("正在采样 {}ms 差分窗口…", window.as_millis());
            } else {
                eprintln!("Sampling a {}ms delta window…", window.as_millis());
            }
        }
        std::thread::sleep(window);
    }
    for round in 0..samples {
        if round > 0 {
            std::thread::sleep(Duration::from_millis(interval.as_millis()));
        }
        let snapshot = collector.snapshot(detail_pid).map_err(anyhow::Error::new)?;
        if kind == "gpu" && snapshot.gpu.is_none() && !enhanced {
            return Err(anyhow::Error::new(CliError::CapabilityUnavailable(
                "GPU utilization is not exposed by this Mac".into(),
            )));
        }
        let mut filtered = match &command {
            Command::Ps {
                pid,
                user,
                sort,
                limit,
            } => filter_processes(snapshot, *pid, user.as_deref(), sort, *limit),
            _ => snapshot,
        };
        privacy.redact_snapshot(&mut filtered);
        if kind == "gpu" && enhanced {
            let power = sample_powermetrics(interval).map_err(anyhow::Error::new)?;
            let data = json!({ "captured_at": filtered.captured_at, "gpu": filtered.gpu,
                "process_coverage": filtered.gpu_process_coverage, "powermetrics": power });
            if format == OutputFormat::Csv {
                return Err(usage(
                    "GPU CSV cannot represent enhanced power data; use JSON/JSONL",
                ));
            }
            print_report(kind, data, format)?;
        } else {
            print_snapshot(kind, &filtered, format, round == 0)?;
        }
    }
    Ok(())
}

fn print_report(kind: &str, data: serde_json::Value, format: OutputFormat) -> Result<()> {
    let envelope = || {
        JsonEnvelope::new(
            kind,
            CapabilityState::Available,
            vec![kind.to_string()],
            data.clone(),
        )
    };
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&envelope())?),
        OutputFormat::Jsonl => println!("{}", serde_json::to_string(&envelope())?),
        OutputFormat::Table => print_report_table(kind, &data),
        OutputFormat::Csv => return Err(usage(format!("CSV is not supported for {kind}"))),
    }
    Ok(())
}

fn print_report_table(kind: &str, data: &serde_json::Value) {
    match kind {
        "disk" => {
            println!("FILESYSTEM\tMOUNT\tTOTAL\tUSED\tAVAILABLE\tCAPACITY");
            for row in data.as_array().map(Vec::as_slice).unwrap_or_default() {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    row["filesystem"].as_str().unwrap_or("--"),
                    row["mountpoint"].as_str().unwrap_or("--"),
                    row["total_bytes"],
                    row["used_bytes"],
                    row["available_bytes"],
                    row["used_percent"]
                        .as_f64()
                        .map(|value| format!("{value:.1}%"))
                        .unwrap_or_else(|| "--".into())
                );
            }
        }
        "network_connections" => {
            println!("PID\tCOMMAND\tPROTOCOL\tENDPOINT\tSTATE");
            if let Some(rows) = data["connections"].as_array() {
                for row in rows {
                    println!(
                        "{}\t{}\t{}\t{}\t{}",
                        row["pid"],
                        row["command"].as_str().unwrap_or("--"),
                        row["protocol"].as_str().unwrap_or("--"),
                        row["endpoint"].as_str().unwrap_or("--"),
                        row["state"].as_str().unwrap_or("--")
                    );
                }
            }
        }
        _ => println!(
            "{}",
            serde_json::to_string_pretty(data).unwrap_or_else(|_| "{}".into())
        ),
    }
}

fn filter_processes(
    mut snapshot: SystemSnapshot,
    pid: Option<i32>,
    user: Option<&str>,
    sort: &str,
    limit: Option<usize>,
) -> SystemSnapshot {
    snapshot.processes.retain(|process| {
        pid.is_none_or(|value| process.pid == value)
            && user.is_none_or(|value| process.user == value)
    });
    match sort {
        "memory" | "mem" => snapshot
            .processes
            .sort_by_key(|process| std::cmp::Reverse(process.resident_bytes.unwrap_or_default())),
        "pid" => snapshot.processes.sort_by_key(|process| process.pid),
        "gpu" => snapshot.processes.sort_by(|left, right| {
            right
                .gpu_percent
                .partial_cmp(&left.gpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        "energy" | "nrg" => snapshot.processes.sort_by(|left, right| {
            right
                .energy_impact
                .unwrap_or(0.0)
                .partial_cmp(&left.energy_impact.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        "power" | "watts" => snapshot.processes.sort_by(|left, right| {
            right
                .power_watts
                .unwrap_or(0.0)
                .partial_cmp(&left.power_watts.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        _ => snapshot.processes.sort_by(|left, right| {
            right
                .cpu_percent
                .partial_cmp(&left.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
    }
    if let Some(limit) = limit {
        snapshot.processes.truncate(limit);
    }
    snapshot
}

fn print_snapshot(
    kind: &str,
    snapshot: &SystemSnapshot,
    format: OutputFormat,
    header: bool,
) -> Result<()> {
    match format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            let data = match kind {
                "processes" => {
                    json!({"captured_at": snapshot.captured_at, "processes": snapshot.processes, "gpu_process_coverage": snapshot.gpu_process_coverage})
                }
                "cpu" => json!({
                    "captured_at": snapshot.captured_at,
                    "cpu": snapshot.cpu,
                    "soc": snapshot.soc,
                    "topology": snapshot.topology,
                }),
                "memory" => json!({
                    "captured_at": snapshot.captured_at,
                    "memory": snapshot.memory,
                    "bandwidth": snapshot.soc.as_ref().map(|soc| json!({
                        "dram_read_gbs": soc.dram_read_gbs,
                        "dram_write_gbs": soc.dram_write_gbs,
                    })),
                }),
                "network" => json!({
                    "captured_at": snapshot.captured_at,
                    "interfaces": snapshot.interfaces,
                    "link": snapshot.link,
                }),
                "gpu" => json!({
                    "captured_at": snapshot.captured_at,
                    "gpu": snapshot.gpu,
                    "process_coverage": snapshot.gpu_process_coverage,
                    "soc": snapshot.soc.as_ref().map(|soc| json!({
                        "gpu_freq_mhz": soc.gpu_freq_mhz,
                        "gpu_watts": soc.power.gpu_watts,
                        "gpu_temp_celsius": soc.temps.gpu_celsius,
                        "gpu_active_percent": soc.gpu_active_percent,
                    })),
                }),
                _ => json!(snapshot),
            };
            let envelope = JsonEnvelope::new(
                kind,
                CapabilityState::Available,
                snapshot.capabilities.clone(),
                data,
            );
            if matches!(format, OutputFormat::Json) {
                println!("{}", serde_json::to_string_pretty(&envelope)?);
            } else {
                println!("{}", serde_json::to_string(&envelope)?);
            }
        }
        OutputFormat::Csv => print!("{}", output::snapshot_csv(kind, snapshot, header)?),
        OutputFormat::Table => print_table(kind, snapshot),
    }
    io::stdout().flush().ok();
    Ok(())
}

fn print_table(kind: &str, snapshot: &SystemSnapshot) {
    match kind {
        "processes" => {
            println!("PID\tCPU%\tMEM\tTHREADS\tUSER\tCOMMAND");
            for process in &snapshot.processes {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    process.pid,
                    process
                        .cpu_percent
                        .map(|v| format!("{v:.1}"))
                        .unwrap_or_else(|| "--".into()),
                    bytes(process.resident_bytes.unwrap_or_default()),
                    process
                        .thread_count
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "--".into()),
                    process.user,
                    process.name
                );
            }
        }
        "cpu" => println!(
            "CPU total={} user={} system={} idle={} load={:?}",
            percent(snapshot.cpu.total_percent),
            percent(snapshot.cpu.user_percent),
            percent(snapshot.cpu.system_percent),
            percent(snapshot.cpu.idle_percent),
            snapshot.cpu.load_average
        ),
        "memory" => println!(
            "MEM used={} total={} free={} wired={} compressed={} pressure={}",
            bytes(snapshot.memory.used_bytes),
            bytes(snapshot.memory.total_bytes),
            bytes(snapshot.memory.free_bytes),
            bytes(snapshot.memory.wired_bytes),
            bytes(snapshot.memory.compressed_bytes),
            percent(snapshot.memory.pressure_percent)
        ),
        "network" => {
            println!("INTERFACE\tRX/s\tTX/s");
            for interface in &snapshot.interfaces {
                println!(
                    "{}\t{}\t{}",
                    interface.name,
                    rate(interface.receive_bytes_per_second),
                    rate(interface.send_bytes_per_second)
                );
            }
        }
        "gpu" => {
            if let Some(gpu) = &snapshot.gpu {
                println!(
                    "GPU {} usage={:.1}% idle={:.1}%",
                    gpu.name.as_deref().unwrap_or("--"),
                    gpu.utilization_percent,
                    gpu.idle_percent
                );
            }
        }
        _ => println!("{kind}: no table renderer yet"),
    }
}

fn run_hardware(
    category: Option<&str>,
    privacy: PrivacyMode,
    format: OutputFormat,
    language: Language,
) -> Result<()> {
    let report = hardware_report_with_privacy(privacy).map_err(anyhow::Error::new)?;
    let data = if let Some(category) = category {
        report
            .sections
            .get(category)
            .cloned()
            .unwrap_or_else(|| json!([]))
    } else {
        json!(report.sections)
    };
    let kind = category.unwrap_or("hardware");
    let envelope = || {
        JsonEnvelope::new(
            kind,
            CapabilityState::Available,
            vec![kind.to_string()],
            data.clone(),
        )
    };
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&envelope())?),
        OutputFormat::Jsonl => println!("{}", serde_json::to_string(&envelope())?),
        OutputFormat::Table => {
            if let Some(object) = data.as_object() {
                println!("{}", language.strings().cli_hardware_categories);
                for key in object.keys() {
                    println!("- {key}");
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&data)?);
            }
        }
        OutputFormat::Csv => return Err(usage("CSV is not supported for hardware")),
    }
    Ok(())
}

/// 硬件页的雷雳 + RDMA 分区（切进硬件页时随分区列表一次性拉取）。
fn transport_sections(language: Language, privacy: PrivacyMode) -> Vec<bmtop_tui::DetailSection> {
    // Device/peer labels can contain personal names. A strict session omits
    // these sections rather than pretending unstructured text is anonymized.
    if privacy == PrivacyMode::Strict {
        return Vec::new();
    }
    let strings = language.strings();
    let switches = bmtop_macos::thunderbolt::read_tb_switches();
    let buses = bmtop_macos::thunderbolt::build_tb_buses(&switches);
    let peers = bmtop_macos::thunderbolt::read_bridge_peers();
    let tb_body = if buses.is_empty() && peers.is_empty() {
        strings.unavailable.to_string()
    } else {
        bmtop_macos::thunderbolt::render_tb_tree(&buses, &peers)
    };
    let rdma = bmtop_macos::rdma::read_rdma_status();
    let mut rdma_body = format!("{}\n", rdma.status);
    for device in &rdma.devices {
        rdma_body.push_str(&format!(
            "\n{}\n  transport {} · state {} · mtu {} · link {}{}",
            device.name,
            device.transport,
            device.port_state,
            device.active_mtu,
            device.link_layer,
            if device.interface.is_empty() {
                String::new()
            } else {
                format!(" · {}", device.interface)
            },
        ));
    }
    vec![
        bmtop_tui::DetailSection::new(strings.section_thunderbolt, tb_body),
        bmtop_tui::DetailSection::new(strings.section_rdma, rdma_body),
    ]
}

/// SoC 一次性采样窗口：不超过 500ms，够 IOReport delta 出数即可。
fn soc_window(interval: RefreshInterval) -> Duration {
    Duration::from_millis(interval.as_millis().min(500))
}

fn soc_probe_json() -> serde_json::Value {
    match bmtop_macos::soc::SocCollector::try_new() {
        Ok(collector) => {
            let probe = collector.probe();
            let groups = [
                "Energy Model",
                "GPU Stats",
                "CPU Stats",
                "Energy Counters",
                "AMC Stats",
            ]
            .iter()
            .enumerate()
            .map(|(bit, name)| {
                json!({
                    "name": name, "enumerated": probe.enumerated_groups & (1 << bit) != 0,
                })
            })
            .collect::<Vec<_>>();
            json!({ "ioreport": probe.ioreport, "smc": probe.smc, "thermal": probe.thermal,
                "initialization": { "state": "available", "reason_code": null },
                "groups": groups,
                "bandwidth": { "state": if probe.enumerated_groups & 16 != 0 { "not_sampled" } else { "unavailable" },
                    "reason_code": if probe.enumerated_groups & 16 != 0 { "delta_sample_required" } else { "amc_group_not_enumerated" },
                    "note": "Group enumeration is not proof that every counter is available; missing metrics remain null." } })
        }
        Err(error) => json!({ "ioreport": false, "smc": false, "thermal": false,
            "initialization": { "state": "unavailable", "reason_code": error.to_string() },
            "groups": [], "bandwidth": { "state": "unavailable", "reason_code": "soc_not_initialized" } }),
    }
}

fn extras_probe_json() -> serde_json::Value {
    let link = bmtop_macos::link::read_link_info();
    let rdma = bmtop_macos::rdma::read_rdma_status();
    json!({
        "battery": bmtop_macos::soc::read_battery().is_some(),
        "disk_io": bmtop_macos::disk_io_available(),
        "ethernet_links": link.ethernet.len(),
        "wifi": link.wifi.is_some(),
        "rdma": rdma.available,
        "fps_permission": bmtop_macos::link::fps_permission_granted(),
    })
}

fn source_state(available: bool) -> &'static str {
    if available {
        "available"
    } else {
        "unavailable"
    }
}

fn doctor_sources(soc: &serde_json::Value, extras: &serde_json::Value) -> serde_json::Value {
    let soc_available = soc["initialization"]["state"].as_str() == Some("available");
    let bool_source = |key: &str| source_state(extras[key].as_bool().unwrap_or(false));
    json!({
        "ioreport": source_state(soc_available),
        "smc": source_state(soc["smc"].as_bool().unwrap_or(false)),
        "thermal": source_state(soc["thermal"].as_bool().unwrap_or(false)),
        "battery": bool_source("battery"),
        "disk_io": bool_source("disk_io"),
        "wifi": bool_source("wifi"),
        "rdma": bool_source("rdma"),
        "fps": bool_source("fps_permission"),
    })
}

fn run_doctor(format: OutputFormat, language: Language) -> Result<()> {
    let term = std::env::var("TERM").unwrap_or_default();
    let term = match term.as_str() {
        "xterm-256color" | "xterm" | "screen" | "screen-256color" | "tmux-256color" | "vt100"
        | "dumb" => term.as_str(),
        "" => "unset",
        _ => "other",
    };
    let soc = soc_probe_json();
    let extras = extras_probe_json();
    let sources = doctor_sources(&soc, &extras);
    let value = json!({
        "schema_version": SCHEMA_VERSION, "kind": "doctor", "version": env!("CARGO_PKG_VERSION"),
        "captured_at": bmtop_core::rfc3339_now(), "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH, "term": term, "tty": io::stdout().is_terminal(),
        "runtime": serde_json::to_value(bmtop_macos::runtime_platform())?,
        "uptime_seconds": uptime_seconds(),
        "soc": soc,
        "extras": extras,
        "sources": sources,
        "commands": {
            "system_profiler": std::path::Path::new("/usr/sbin/system_profiler").is_file(),
            "powermetrics": std::path::Path::new("/usr/bin/powermetrics").is_file(),
            "lsof": std::path::Path::new("/usr/sbin/lsof").is_file(),
        },
        "keyboard": { "base_keys": true, "command_digit": keyboard_enhancement_supported(),
            "note": language.strings().cli_keyboard_note },
        "privacy": { "hardware_identifiers_collected": false, "process_arguments_collected": false,
            "screen_recording_requested": false },
    });
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&value)?),
        OutputFormat::Jsonl => println!("{}", serde_json::to_string(&value)?),
        OutputFormat::Table => {
            println!(
                "platform={} arch={} tty={} system_profiler={} powermetrics={}",
                value["platform"],
                value["arch"],
                value["tty"],
                value["commands"]["system_profiler"],
                value["commands"]["powermetrics"]
            );
            println!("SoC: {}", value["soc"]["initialization"]);
            println!("Bandwidth: {}", value["soc"]["bandwidth"]);
        }
        OutputFormat::Csv => return Err(usage("CSV is not supported for doctor")),
    }
    Ok(())
}

fn completion(shell: Shell, language: Language) -> Result<()> {
    // 带上帮助文案：zsh / fish 的补全菜单会把 about 显示在候选项旁边。
    let mut command = localized_help(Cli::command(), language.strings());
    let name = "bmtop";
    match shell {
        Shell::Bash => generate(shells::Bash, &mut command, name, &mut io::stdout()),
        Shell::Fish => generate(shells::Fish, &mut command, name, &mut io::stdout()),
        Shell::Zsh => generate(shells::Zsh, &mut command, name, &mut io::stdout()),
    }
    Ok(())
}

fn parse_interval(value: &str) -> Result<RefreshInterval> {
    let (number, multiplier) = if let Some(value) = value.strip_suffix("ms") {
        (value, 1)
    } else if let Some(value) = value.strip_suffix('s') {
        (value, 1_000)
    } else {
        return Err(usage("interval must use ms or s, for example 500ms or 1s"));
    };
    let number: u64 = number
        .parse()
        .map_err(|_| usage("invalid interval number"))?;
    RefreshInterval::from_millis(number.saturating_mul(multiplier))
        .map_err(|error| usage(error.to_string()))
}

/// sysexits 风格：64 用法错误 / 69 能力不可用 / 77 权限被拒 / 70 其他。
fn exit_code(error: &anyhow::Error) -> i32 {
    if let Some(cli) = error.downcast_ref::<CliError>() {
        return match cli {
            CliError::Usage(_) => 64,
            CliError::CapabilityUnavailable(_) => 69,
        };
    }
    if let Some(collector) = error.downcast_ref::<CollectorError>() {
        return match collector {
            CollectorError::AuthorizationDenied => 77,
            CollectorError::UnsupportedPlatform => 69,
            _ => 70,
        };
    }
    70
}
fn percent(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.1}%"))
        .unwrap_or_else(|| "--".into())
}
fn rate(value: Option<f64>) -> String {
    value
        .map(|value| format!("{} /s", bytes(value as u64)))
        .unwrap_or_else(|| "--".into())
}
fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut number = value as f64;
    let mut index = 0;
    while number >= 1024.0 && index < UNITS.len() - 1 {
        number /= 1024.0;
        index += 1;
    }
    if index == 0 {
        format!("{}{}", value, UNITS[index])
    } else {
        format!("{number:.1}{}", UNITS[index])
    }
}
fn signal_name(signal: ProcessSignal) -> &'static str {
    match signal {
        ProcessSignal::Terminate => "TERM",
        ProcessSignal::Kill => "KILL",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmtop_core::ProcessRow;

    #[test]
    fn parse_interval_accepts_ms_and_s_within_bounds() {
        assert_eq!(parse_interval("250ms").unwrap().as_millis(), 250);
        assert_eq!(parse_interval("2s").unwrap().as_millis(), 2_000);
        for invalid in ["1", "1m", "abcms", "-1s", "100ms", "61s", ""] {
            assert!(parse_interval(invalid).is_err(), "{invalid} should fail");
        }
    }

    #[test]
    fn exit_codes_come_from_error_types_not_message_text() {
        assert_eq!(exit_code(&usage("bad flag")), 64);
        assert_eq!(
            exit_code(&anyhow::Error::new(CliError::CapabilityUnavailable(
                "no GPU".into()
            ))),
            69
        );
        assert_eq!(
            exit_code(&anyhow::Error::new(CollectorError::AuthorizationDenied)),
            77
        );
        // 措辞里带 "permission" 的普通错误不再被误判成 77。
        assert_eq!(exit_code(&anyhow::anyhow!("permission wording only")), 70);
    }

    fn snapshot_with(processes: Vec<ProcessRow>) -> SystemSnapshot {
        SystemSnapshot {
            captured_at: bmtop_core::rfc3339_now(),
            captured_at_display: "00:00:00".into(),
            cpu: Default::default(),
            memory: Default::default(),
            processes,
            interfaces: Vec::new(),
            gpu: None,
            capabilities: vec!["processes".into()],
            gpu_process_coverage: None,
            uptime_seconds: None,
            soc: None,
            topology: None,
            battery: None,
            disk_io: None,
            link: None,
            fps: None,
        }
    }

    fn row(pid: i32, user: &str, cpu: f64, resident: u64) -> ProcessRow {
        ProcessRow {
            pid,
            parent_pid: 1,
            uid: 501,
            user: user.into(),
            name: format!("proc{pid}"),
            path: None,
            state: "run".into(),
            resident_bytes: Some(resident),
            virtual_bytes: None,
            thread_count: None,
            file_descriptor_count: None,
            cpu_percent: Some(cpu),
            gpu_percent: None,
            cpu_time_seconds: None,
            energy_impact: Some(cpu),
            power_watts: Some(cpu / 10.0),
            start_time_seconds: 0,
            start_time_microseconds: 0,
            disk_read_bytes: None,
            disk_written_bytes: None,
            arguments: None,
            threads: None,
        }
    }

    #[test]
    fn filter_processes_filters_sorts_and_limits() {
        let snapshot = snapshot_with(vec![
            row(1, "a", 5.0, 100),
            row(2, "b", 50.0, 10),
            row(3, "a", 25.0, 999),
        ]);
        let by_cpu = filter_processes(snapshot.clone(), None, None, "cpu", None);
        assert_eq!(
            by_cpu.processes.iter().map(|p| p.pid).collect::<Vec<_>>(),
            vec![2, 3, 1]
        );
        let by_memory = filter_processes(snapshot.clone(), None, None, "memory", Some(1));
        assert_eq!(by_memory.processes.len(), 1);
        assert_eq!(by_memory.processes[0].pid, 3);
        let by_user = filter_processes(snapshot.clone(), None, Some("a"), "pid", None);
        assert_eq!(
            by_user.processes.iter().map(|p| p.pid).collect::<Vec<_>>(),
            vec![1, 3]
        );
        let by_pid = filter_processes(snapshot, Some(2), None, "cpu", None);
        assert_eq!(by_pid.processes.len(), 1);
    }
}
