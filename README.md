# bmtop

`bmtop` is a local-first macOS terminal monitor for process, CPU, memory, network, disk, GPU, hardware and sensor information. On Apple Silicon it also reads SoC metrics natively — E/P cluster frequency and residency, CPU/GPU/ANE/DRAM power, system wall power, temperatures, fans and thermal pressure — via IOReport/SMC with no sudo required. It further collects battery state, system-wide disk I/O rates, per-process GPU usage, network link type (Ethernet speed / Wi-Fi generation), Thunderbolt topology, RDMA status, GPU peak frequency with theoretical TFLOPS, and — opt-in via the `f` key, requiring Screen Recording permission — display FPS. DRAM/ANE bandwidth is shown where the kernel exposes AMC byte counters (some macOS 26 builds reject that IOReport group; the rows hide there).

## Install

### Homebrew

```sh
brew tap bettermacnet/tap
brew install bmtop
```

Or in one step: `brew install bettermacnet/tap/bmtop`. The formula builds
from source (Homebrew installs the Rust toolchain as a build-only
dependency).

### Prebuilt binary

Each release ships a signed-checksum universal binary (arm64 + x86_64):

```sh
curl -sLO https://github.com/BetterMacNet/bmtop/releases/latest/download/bmtop-macos-universal.tar.gz
tar -xzf bmtop-macos-universal.tar.gz
./bmtop --version
```

Verify the download against the `.sha256` file published next to the asset.

## Build

```sh
export CARGO_HOME="$PWD/.cargo-cache"
cargo test --workspace
cargo run -p bmtop -- doctor --format json
cargo run -p bmtop -- ps --format json
```

The default command opens the interactive TUI. In a pipeline, use an explicit
subcommand and `--format json`, `jsonl`, or `csv`:

```sh
bmtop top
bmtop ps --sort memory --limit 10 --format table
bmtop memory --format json
bmtop network -n 60 -i 1s --format jsonl   # 60 samples, then exit
bmtop hardware --format json
```

JSON output follows `schema_version` 2: `captured_at` is UTC RFC 3339 and
`capabilities` lists the capabilities of the snapshot that produced the data.
`-n/--count N` samples N times and exits; `--watch` alone runs until Ctrl-C.
Exit codes are sysexits-style: 64 usage, 69 capability unavailable,
77 permission denied, 70 other failures.

## Interaction

Use `1` through `9` / `F1` through `F9` to jump to a mode, `←`/`→` (or
`Tab`/`Shift+Tab`) to cycle through the mode bar, `↑`/`↓` to move, `/` to
search, `Space` to pause, `?` for help and `q` to leave.

The following keys follow the system `top` conventions: `o` cycles the sort
column (CPU/MEM/PID) and `O` reverses the order; `s` opens a prompt for the
sampling interval (bare digits mean seconds, `500ms` also works, blank keeps
the current value — digits typed inside the prompt do not switch modes);
`u` filters by user (blank or Esc shows all).

Linux top (procps) conventions are covered too: `P`/`M`/`N` sort directly by
CPU/memory/PID and `R` reverses; `d` is an alias for `s`; `k` opens the
terminate prompt (same as `x`); `c` toggles full command paths; `i` hides
idle processes (rows whose CPU is exactly 0; unknown-CPU rows stay); `V`
switches the process table to a tree indented by parent PID; `H` switches the
detail pane to the selected process's thread list (per-thread CPU, state,
name — collected only for the selected process). `+`/`-` step the interval by
250ms and `Ctrl+L` forces a full redraw. The cursor is pinned to the process
under it, not the row number, so re-sorting between samples does not move it.
Note: `j`/`k` no longer move the selection; `k` now kills, use `↑`/`↓`.
When the terminal supports the enhanced keyboard protocol (kitty, WezTerm,
Ghostty, …) bmtop enables it on startup, so `Command+1` through `Command+9`
also switch modes; `bmtop doctor` reports the detection result as
`keyboard.command_digit`. The default macOS Terminal does not support it, and
terminal tab shortcuts may intercept those keys first.

The process table is read-only until `x` or `X` is chosen. Termination always
requires confirmation and revalidates PID plus process start time. PID 0, PID 1
and the bmtop process itself are protected.

## Permission boundary

Normal collection does not require administrator access. SoC metrics
(cluster frequencies, power, temperatures, fans) come from the private
IOReport library, read-only SMC keys and `notify_get_state` — all readable
without root. On Intel Macs or if IOReport is unavailable, `soc` data is
omitted and the UI degrades to the previous view; `bmtop doctor` reports the
probe result under `soc`. `bmtop gpu --enhanced`
and `bmtop sensors --enhanced` run one `powermetrics` sample through
`/usr/bin/sudo` (fixed binary, fixed arguments, no shell) and merge GPU
frequency/power and thermal pressure into the output; `--enhanced` is rejected
elsewhere. Signals for another user's process also go through `sudo` only when
explicitly requested. The main TUI never runs as root and passwords are never
stored.

Hardware identifiers are redacted in JSON by default. Use
`--show-sensitive` only when the local output is trusted.

## Universal build

`scripts/build-universal.sh` builds `arm64` and `x86_64` slices and combines
them with `lipo`. It requires both Rust targets to already be installed; it
does not mutate the global Rust toolchain.


## License

`bmtop` is licensed under the [MIT License](LICENSE).

Copyright (c) 2026 BetterMacNet

## Third-party notices

`bmtop` statically links a number of Rust crates, all under permissive
licenses (MIT, Apache-2.0, and similar). The full list of bundled
dependencies, their versions, licenses and upstream repositories is kept in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md), as required by those
licenses. SoC metrics are read through Apple system libraries (IOReport,
IOKit/SMC), which ship with macOS and are not redistributed.
