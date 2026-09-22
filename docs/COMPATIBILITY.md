# Capability probes and compatibility validation

This change does NOT certify a Mac/OS compatibility matrix. Linux support is for
pure Rust/CLI tests and structural tooling only; native metric commands return
UnsupportedPlatform (exit 69), never fabricated monitoring data.

`bmtop doctor --format jsonl` is a one-shot capability diagnostic. The legacy
`doctor.soc.ioreport/smc/thermal` booleans remain, with additive initialization
state/reason_code, enumerated groups, and bandwidth state. Initialization failure
means the collector was not initialized: false smc/thermal does not independently
prove SMC/thermal hardware absence.

| reason_code | Observed stage, not an inferred root cause |
|---|---|
| unsupported_platform | non-macOS backend |
| collector_already_active | process-local IOReport collector lease already held |
| energy_channels_unavailable | Energy Model channel query returned no dictionary |
| channel_allocation_failed | mutable channel dictionary could not be created |
| subscription_failed | merged channel subscription creation failed |
| initialization_failed | other native initialization result |
| amc_group_not_enumerated | AMC Stats query returned no group during initialization |
| delta_sample_required | group was enumerated; a live delta is still required |
| soc_not_initialized | no SoC session from which to query bandwidth |

Group enumeration does NOT mean every channel is usable. Subscription failure is
not automatically labeled "permission denied" or "kernel rejected": neither is
proven by that return code. Missing metrics remain null; nonfinite temperatures
and power readings are rejected. No speculative SoC family fallback table is used.

The C SoC implementation uses process-global state. An atomic lease disallows a
second collector, and the Rust handle is not Sync. The TUI constructs SoC state
on its sampler thread; details do not construct another sampler. Direct calls to
the private C entrypoints bypass Rust's lease and remain unsupported.

GPU `gpu_process_coverage` is additive to schema v2. Counts refer to enumerated
GPU-active client records, not unique PIDs or proof of coverage across protected
processes. Multiple clients are aggregated per PID in Rust; PID plus start time
keys avoid reusing another process's prior counter. Up to 16384 clients/four
attempts are retained. Truncation or unavailability invalidates the GPU delta
baseline and leaves process GPU percentages unavailable instead of normalizing
an incomplete enumeration into a whole-system total. GPU model names are now
best-effort direct IORegistry metadata; they may be absent on some machines.

## Hardware acceptance matrix (all pending real-device verification)

| Device class | Core/CLI | SoC/GPU | SMC/thermal | FPS | Status |
|---|---|---|---|---|---|
| Intel Mac | pending | unsupported metrics must degrade | pending | opt-in only | not tested |
| Apple Silicon older generations | pending | pending | pending | opt-in only | not tested |
| Apple Silicon newer generations | pending | pending | pending | opt-in only | not tested |
| No battery / fanless device | pending | pending | distinguish missing vs zero | opt-in only | not tested |

Record exact OS version/build, chip family, sample window, commit/source hash,
normal/enhanced privilege mode, probe JSON and independent reference window for
each real test. Never substitute these synthetic unit fixtures for captured
IOReport group fixtures. Public reports should omit serials/UUIDs and argv.
