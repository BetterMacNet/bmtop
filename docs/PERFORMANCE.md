# Performance methodology and regression tools

Script unit tests use synthetic inputs and are not benchmarks of bmtop.
`test_native_contract.py` validates production C bounds through a mocked IOKit
adapter, not real macOS performance or ABI compatibility.

The TUI has separate sampler and detail lanes with one-slot request queues.
Native sampler ownership is exclusive; slow system_profiler/lsof/df/mount work
uses bounded subprocess helpers. Commands have a wall-clock timeout (15s profiler,
5s lsof, 3s df/mount/ps/bridge/RDMA), stdout limit 16 MiB and stderr limit 64 KiB.
The timeout kills only the dedicated child process group, then reaps it. These
bounds apply to each command, not a combined deadline for a multi-command page.
Authorization prompts/powermetrics and direct IOKit C calls are not forcibly
cancelled. Closing the TUI cancels queued work/discards results; a running native
query may continue until return/timeout. It no longer blocks the sampler lane.

Rendering is invalidated by data/detail/input/resize rather than every loop.
Crossterm has no integrated wakeup for std mpsc: while work is in flight, the loop
polls at 40ms; otherwise it waits for the next sample or user input (60s maximum
while paused). It is reduced redraw, not a claim of zero wakeups. Startup,
mouse/keyboard event bursts and slow calls can still increase cost.

`--diagnostics-file PATH` records queue/service/detail/render duration summaries,
samples/errors/discarded details/renders/input/overruns. Each timing series keeps
at most 2048 observations; total counts are lifetime counts, quantiles are for
that retained window. No process or hardware identity is included. Files are
exclusive-create, private and written on normal exit. Fatal errors or forced
termination do not produce a complete report.

## Run and compare on one Mac

Build release binaries with the same profile/toolchain, interval, terminal size,
permissions and power/thermal state. Run each scenario repeatedly; warm the
machine with a separate run before collecting comparisons. The provided PTY
harness explicitly includes startup/shutdown; it does not secretly subtract them.

```sh
cargo build --release --locked -p bmtop
python3 scripts/benchmark.py --seconds 30 --interval 5s --mode 1 --output baseline.json
# Rebuild the candidate at the same binary path before the comparison:
python3 scripts/benchmark.py --seconds 30 --interval 5s --mode 1 --baseline baseline.json --max-regression-pct 20 --output candidate.json
```

The harness sets 160×48, strict privacy and no FPS, counts but never stores terminal
bytes, samples only the bmtop PID's RSS with ps every 500ms, sends q, and reaps the
child. `cpu_seconds_wait4` is OS resource accounting; it may include waited-for
child tools and is not an exclusive renderer/collector CPU metric. RSS ps units
are converted from KiB to bytes. The observer itself creates small overhead.

The render gate checks draws against sample/detail/input activity; it also rejects
collector errors or fewer than two samples. Relative CPU/RSS comparison requires
matching platform/scenario/diagnostics version, rejects zero/missing baselines as
inconclusive, and applies the configured tolerance. Hosted-runner measurements
are observational, not a stable performance SLA. Do not compare runs from
different Macs merely because architecture and OS labels happen to match.

For rigorous accuracy/overhead acceptance, independently measure wakeups and
exclusive process CPU using Instruments, and compare identical workload windows
with system counters or powermetrics. Test 250ms/1s/5s/60s; increase --seconds to at
least two intervals plus two seconds. Test many processes, GPU load, disk/network
load, Hardware queries, pause/resume and FPS separately. Record package energy vs
estimated process power distinctly. No automatic permission prompt in CI.
