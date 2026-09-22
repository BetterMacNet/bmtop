# Getting started

bmtop is a local macOS terminal monitor. It reads system resources without a
daemon or network upload. Normal collection does not need administrator access.

## Install

With Homebrew:

```sh
brew install bettermacnet/tap/bmtop
```

Or install the universal release binary from the
[latest release](https://github.com/BetterMacNet/bmtop/releases/latest), verify
its `.sha256` file, and put `bmtop` on your `PATH`.

## First run

```sh
bmtop
```

Use `1`–`9` or `←`/`→` to change pages, `↑`/`↓` to select a process, `/` to
search, `?` for help and `q` to quit. `o` changes the process sort column; `s`
asks for a sampling interval; `x`/`k` sends SIGTERM after confirmation.

## Useful commands

```sh
# Check what this Mac can expose
bmtop doctor --format json

# Inspect top consumers
bmtop ps --sort energy --limit 10
bmtop ps --sort memory --limit 10

# Produce bounded data for scripts
bmtop memory --format json
bmtop network --count 10 --interval 1s --format jsonl
bmtop memory --count 10 --interval 1s --format csv
```

JSON schema version 2 includes `captured_at` and a capability list. Optional
metrics are reported as unavailable rather than fabricated as zero. See the
[output contract](OUTPUT-CONTRACT.md) for formats and exit codes.

## Optional data

Apple Silicon SoC, sensor and GPU details are best-effort native metrics. Intel
Macs and systems without a readable source simply omit unavailable fields. The
`f` key enables FPS collection only after the user grants Screen Recording
permission.

`bmtop gpu --enhanced` and `bmtop sensors --enhanced` run one explicit
`powermetrics` sample through `sudo`. This is not used by the TUI or the normal
path. Review the [privacy boundaries](PRIVACY.md) before exporting diagnostics.

## Help and reports

- [README](../README.md) — product overview and full keybinding reference
- [Compatibility](COMPATIBILITY.md) — capability boundaries and test status
- [Bug report](https://github.com/BetterMacNet/bmtop/issues/new?template=bug_report.md)
- [Feature request](https://github.com/BetterMacNet/bmtop/issues/new?template=feature_request.md)
- [Discussions](https://github.com/BetterMacNet/bmtop/discussions)
- [Security policy](../SECURITY.md)
