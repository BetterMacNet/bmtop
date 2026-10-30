# bmtop 改进版源码交付

版本：`0.2.2`。本页记录功能边界和验证要求；仍不等于所有硬件型号都完成验收。原工程的 MIT 许可证、四个 crate、资源和发布脚本保留。

## 先看验证边界

代码验证已覆盖：Rust 格式检查、Clippy、工作区测试、doctor smoke check、Python 辅助测试、GPU/磁盘 C 合约测试和 C/Rust FFI 布局契约。真实运行记录不包含快照、用户路径或凭据；这些测试不替代全部 Mac 硬件验收。

## 已落地功能

| 工作项 | 实施结果 | 主要文件 |
|---|---|---|
| 核心采样/慢详情分离 | 两个专用线程、容量为 1 的请求通道、独立 in-flight 状态；SoC 只归采样线程所有 | `bmtop-tui/src/worker.rs`、`lib.rs` |
| 防止过期详情覆盖 | 页面和强制刷新版本号校验；支持 A→B→A；旧任务完成后立即调度当前页 | `worker.rs`、`lib.rs` |
| 按需重绘 | 新快照/详情/按键/窗口大小才重绘；空闲等待采样截止时间，不再每 80ms 无条件 draw | `scheduling.rs`、`lib.rs` |
| 子进程约束 | 固定程序和参数、独立进程组、墙钟超时、输出容量上限；超时清理子进程组 | `bmtop-macos/src/command.rs` |
| 统一隐私策略 | `strict/default/off`，system_profiler 闭合字段白名单，未知字段及动态键名丢弃 | `bmtop-core/src/privacy.rs` |
| 持久配置 | 配置版本、路径搜索、优先级、类型/范围验证、`--print-config`、禁止静默回写 | `bmtop/src/config.rs` |
| GPU 进程记录 | 256 起步动态增长，最多 16384 条/4 次枚举；明确 coverage/truncated；历史加入进程启动时间 | `gpu_buffer.rs`、`bmtop_sys.c`、`lib.rs` |
| 网络连接修复 | 按 lsof 文件描述符而非 PID 收尾；保留同一进程多个连接；支持 IPv6/NUL 记录 | `connections.rs` |
| CLI 输出 | 连接和硬件 count/watch 生效；doctor JSONL 单行；CSV 增加 CPU/内存/GPU/磁盘/连接，流中表头仅一次 | `main.rs`、`output.rs` |
| 单次采样体验 | `--quick` 250ms，TTY stderr 提示，不污染 stdout；`--schema-version` | `main.rs` |
| 能力诊断 | SoC 初始化阶段 reason_code、IOReport group 枚举情况；非有限值过滤 | `soc.rs`、`bmtop_soc.c`、`doctor` |
| 测量与工程协作 | 有界时序统计、退出时导出、PTY 基准/回归工具、测试、CI、贡献/安全/Issue 模板 | `diagnostics.rs`、`scripts/`、`.github/` |

上述“已落地”表示代码已写入并通过自动化检查，不等于所有 Mac 型号完成验收。

## 在 Mac 上首次验证

需要 Xcode Command Line Tools、Rust stable（含 rustfmt、clippy）；辅助脚本建议 Python 3.11+。依赖使用现有 `Cargo.lock`，未 vendor，第一次构建仍需能访问已有依赖的下载源。

```sh
cd bmtop
./scripts/verify.sh
cargo build --release --locked -p bmtop
./target/release/bmtop --config config.example.toml
```

`verify.sh` 按顺序执行格式检查、Clippy `-D warnings`、工作区测试、doctor smoke check、Python 测试和 C 合约测试。任一步失败即停止。未完成的是不同 macOS/硬件型号的兼容矩阵。

## 新功能使用

```sh
# 可持久复用的配置文件；不必写入用户目录
./target/release/bmtop --config config.example.toml --print-config
./target/release/bmtop --no-config --schema-version
BMTOP_INTERVAL=5s ./target/release/bmtop top --mode 2

# 本机安全诊断；doctor 保留兼容的顶层结构
./target/release/bmtop --no-config doctor --format jsonl

# 连接模式确实输出两份完整 JSONL 文档；普通用户只能看到可读取的连接
./target/release/bmtop network --connections --count 2 --privacy strict --format jsonl
./target/release/bmtop cpu --quick --format json
./target/release/bmtop memory --count 3 --format csv

# timing.json 必须不存在；正常 q 退出后写入，不含快照/命令行/序列号
./target/release/bmtop top --diagnostics-file timing.json

# 真实 Mac 测量，包含启动成本；不会打开 FPS、sudo 或网络服务
python3 scripts/benchmark.py --binary target/release/bmtop --seconds 30 --interval 5s --output baseline.json
python3 scripts/benchmark.py --binary target/release/bmtop --seconds 30 --interval 5s --baseline baseline.json --output current.json
```

`--quick` 不是瞬时精确测量，也不是缓存：仍执行真实预热/差分窗口。默认 CPU/GPU 单次窗口保留原实现的 `min(interval, 500ms)`。传感器命令的 system_profiler 耗时不包含在这 250ms 内。

## 没有宣称完成的部分

| 建议 | 当前边界 |
|---|---|
| IOReport/SMC 全机型兼容矩阵 | 完成探测和故障阶段区分；没有真实 M 系列/Intel/OS 版本矩阵实测，未添加猜测性机型兜底 |
| 完整任务取消 | 排队任务在关闭时取消，过期结果丢弃；运行中的 C/IOKit 调用不强制中断，慢系统命令靠超时收敛。快速切页可能仍需等待当前详情结束，但不阻塞采样 |
| 性能提升幅度 | 提供测量及回归工具，没有真实 bmtop CPU/RSS/p95 前后对照结果 |
| GitHub Issues/安全渠道 | 添加模板和政策；是否允许公开 Issue、是否开启私密报告仍需仓库维护者设置 |
| 配置完整度 | 持久启动偏好已实现；未实现运行时保存、主题、任意列/布局编辑 |
| 国际化/导出生态 | 保留中英双语；未实现新语言资源框架、OpenMetrics、历史回放、远程/API/插件 |
| 发布与安装 | 本次源码提交会发布 `v0.2.2`；universal 二进制和 Homebrew 公式由 release workflow 生成/更新 |

## Mac 验收重点

在支持的每一类实机上，先运行完整验证脚本，再观察 250ms/1s/5s/60s 四档；打开 Hardware 同时确认采样仍推进；反复切 8→6→8 和强制刷新，确认旧详情不覆盖当前页；暂停/恢复/窗口缩放/信号确认后终端应恢复正常。

核对普通账户、IOReport 不可用、GPU 记录缺失/截断、无电池机器、外接磁盘、IPv6、多连接进程及长时间运行。FPS 只在用户明确按键及系统授权下测试；不为通过测试而自动申请权限。保持原有受保护 PID、启动时间检查和 sudo 边界。

完整说明：`docs/PRIVACY.md`、`docs/CONFIGURATION.md`、`docs/COMPATIBILITY.md`、`docs/OUTPUT-CONTRACT.md`、`docs/PERFORMANCE.md`。
