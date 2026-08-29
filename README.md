<h1 align="center">bmtop</h1>

<p align="center">
  <b>A local-first terminal monitor for macOS.</b><br/>
  htop-style process control plus native Apple Silicon SoC telemetry — power, frequency,
  temperature, fans — with no sudo and no daemons.
</p>

<p align="center">
  <a href="https://github.com/BetterMacNet/bmtop/actions/workflows/ci.yml"><img src="https://github.com/BetterMacNet/bmtop/actions/workflows/ci.yml/badge.svg" alt="CI"/></a>
  <a href="https://github.com/BetterMacNet/bmtop/releases/latest"><img src="https://img.shields.io/github/v/release/BetterMacNet/bmtop" alt="Latest release"/></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT license"/></a>
  <img src="https://img.shields.io/badge/platform-macOS%20(Apple%20Silicon%20%7C%20Intel)-black" alt="Platform"/>
  <img src="https://img.shields.io/badge/built%20with-Rust-orange" alt="Rust"/>
</p>

<p align="center">
  <img src="assets/overview.png" alt="bmtop overview — native SoC telemetry on Apple Silicon" width="920"/>
</p>

## Highlights

- **Native SoC telemetry on Apple Silicon** — E/P cluster frequency and residency,
  CPU/GPU/ANE/DRAM power, system wall power, temperatures, fans and thermal
  pressure, read directly via IOReport/SMC. No `sudo`, no `powermetrics` daemon.
- **Everything in one dashboard** — processes, CPU, memory, network, disk I/O,
  GPU (including per-process usage and peak frequency with theoretical TFLOPS),
  battery, hardware and sensors.
- **Deep hardware awareness** — network link type (Ethernet speed / Wi-Fi
  generation), Thunderbolt topology, RDMA status, and opt-in display FPS.
- **Scripting-grade output** — every panel is also a subcommand with stable
  `json` / `jsonl` / `csv` output, versioned schema and sysexits-style exit codes.
- **Local-first and privacy-conscious** — no telemetry, no network calls;
  hardware identifiers are redacted in JSON by default.
- **top/htop muscle memory** — sorting, filtering, tree view and kill flows
  follow macOS `top` and procps `top` conventions.

<p align="center">
  <img src="assets/processes.png" alt="bmtop process view — htop-style table with per-process details" width="920"/>
</p>

## Install

### Homebrew

```sh
brew install bettermacnet/tap/bmtop
```

### Prebuilt binary

Each release ships a universal binary (arm64 + x86_64):

```sh
curl -sLO https://github.com/BetterMacNet/bmtop/releases/latest/download/bmtop-macos-universal.tar.gz
tar -xzf bmtop-macos-universal.tar.gz
./bmtop
```

Verify the download against the `.sha256` file published next to the asset.

### From source

```sh
git clone https://github.com/BetterMacNet/bmtop.git
cd bmtop
cargo install --path crates/bmtop
```

## Usage

Run `bmtop` to open the interactive TUI. In a pipeline, use an explicit
subcommand and a machine-readable format:

```sh
bmtop top                                  # one-shot table snapshot
bmtop ps --sort memory --limit 10          # top memory consumers
bmtop memory --format json                 # single JSON snapshot
bmtop network -n 60 -i 1s --format jsonl   # 60 samples, then exit
bmtop gpu --enhanced                       # one sudo powermetrics sample merged in
bmtop doctor --format json                 # capability probe report
```

JSON output follows `schema_version` 2: `captured_at` is UTC RFC 3339 and
`capabilities` lists what the producing snapshot could observe. `-n/--count N`
samples N times and exits; `--watch` runs until Ctrl-C. Exit codes are
sysexits-style: `64` usage, `69` capability unavailable, `77` permission
denied, `70` other failures.

Shell completions for bash, zsh and fish are installed by Homebrew, or can be
generated with `bmtop completion <shell>`.

## Keybindings

### Navigation

| Key | Action |
|-----|--------|
| `1`–`9` / `F1`–`F9` | Jump to a mode |
| `←` `→` / `Tab` `Shift+Tab` | Cycle through the mode bar |
| `↑` `↓` | Move selection |
| `/` | Search |
| `Space` | Pause |
| `?` | Help |
| `q` | Quit |

When the terminal supports the enhanced keyboard protocol (kitty, WezTerm,
Ghostty, …) bmtop enables it on startup, so `Command+1`–`Command+9` also
switch modes; `bmtop doctor` reports the detection result as
`keyboard.command_digit`.

### Sorting and filtering (macOS `top` conventions)

| Key | Action |
|-----|--------|
| `o` / `O` | Cycle sort column (CPU/MEM/PID) / reverse order |
| `s` | Prompt for sampling interval (`2` = seconds, `500ms` works too) |
| `u` | Filter by user (blank or `Esc` shows all) |

### procps `top` conventions

| Key | Action |
|-----|--------|
| `P` / `M` / `N` | Sort by CPU / memory / PID |
| `R` | Reverse sort order |
| `d` | Alias for `s` |
| `c` | Toggle full command paths |
| `i` | Hide idle processes |
| `V` | Tree view indented by parent PID |
| `H` | Thread list for the selected process |
| `+` / `-` | Step interval by 250ms |
| `Ctrl+L` | Force full redraw |

The cursor is pinned to the process under it, not the row number, so
re-sorting between samples never moves your selection.

### Process actions

| Key | Action |
|-----|--------|
| `x` / `k` | Terminate the selected process (SIGTERM, with confirmation) |
| `X` | Force-kill the selected process (SIGKILL, with confirmation) |
| `f` | Toggle display FPS (requires Screen Recording permission) |

Termination always requires confirmation and revalidates PID plus process
start time. PID 0, PID 1 and the bmtop process itself are protected.

## Permissions and privacy

Normal collection needs **no administrator access**. SoC metrics come from the
private IOReport library, read-only SMC keys and `notify_get_state` — all
readable without root. On Intel Macs, or when IOReport is unavailable, `soc`
data is omitted and the UI degrades gracefully; `bmtop doctor` reports the
probe result under `soc`.

`bmtop gpu --enhanced` and `bmtop sensors --enhanced` run one `powermetrics`
sample through `/usr/bin/sudo` (fixed binary, fixed arguments, no shell) and
merge GPU frequency/power and thermal pressure into the output; `--enhanced`
is rejected elsewhere. Signals for another user's process also go through
`sudo` only when explicitly requested. The TUI never runs as root and
passwords are never stored.

Hardware identifiers are redacted in JSON by default; use `--show-sensitive`
only when the local output is trusted. DRAM/ANE bandwidth rows appear where
the kernel exposes AMC byte counters (some macOS 26 builds reject that
IOReport group; the rows hide there).

## Development

```sh
./scripts/verify.sh            # fmt, clippy -D warnings, all tests, doctor smoke check
./scripts/build-universal.sh   # arm64 + x86_64 slices combined with lipo
./scripts/package-release.sh   # universal binary + tar.gz + sha256 in dist/
```

The workspace is split into `bmtop-core` (models, schema, i18n),
`bmtop-macos` (native collectors), `bmtop-tui` (ratatui UI) and `bmtop`
(CLI). Releases are cut by pushing a `v*` tag; CI validates the tag against
the workspace version, builds the universal binary and publishes the GitHub
release automatically.

## License

[MIT](LICENSE) © 2026 BetterMacNet

Bundled third-party crates are listed with their licenses in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md). SoC metrics are read
through Apple system libraries (IOReport, IOKit/SMC), which ship with macOS
and are not redistributed.
