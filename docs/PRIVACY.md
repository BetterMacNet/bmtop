# Privacy boundaries

No telemetry, upload, listening server, daemon or cloud integration is added.
Policies are explicit; **none is a guarantee of perfect anonymity**.

| Field / surface | default | strict | off |
|---|---|---|---|
| system_profiler unknown fields or dynamic key names | removed | removed | unchanged |
| Reviewed serial/UUID/UDID/address/name identifiers | redacted | redacted | unchanged |
| Reviewed hardware measurements | retained | retained; selected model/manufacturer labels masked | unchanged |
| Process argv | removed | removed | retained when collected for selected PID |
| Process names, usernames, paths, thread names | retained | masked/removed | retained |
| Connection command / endpoint | retained | null | retained |
| Non-root mount labels / remote share filesystem names | retained | masked | retained |
| PID, UID, start time, timestamps, numeric usage, interface names | retained | retained | retained |
| Unstructured Thunderbolt/RDMA detail sections in TUI | retained | omitted | retained |

`--show-sensitive` is a compatibility alias for `--privacy off`. It conflicts with
an explicitly supplied `--privacy` (including BMTOP_PRIVACY via clap). Never paste
an off-mode report into a public issue. Off shows a warning on stderr only when
stderr is a TTY; scripts remain non-interactive. This is an intentional expansion
of the old hardware-only flag: selected argv can also be exposed.

The default profiler policy has changed from keyword blacklisting to a reviewed
field allowlist. A legitimate new Apple field can disappear until reviewed. This
is fail-closed behavior, not proof that the field is unsupported by the hardware.
Unknown object keys are dropped because identifiers can be used as dictionary
keys. Known identifier names retain the `••••••` marker for recognition.
Containers recurse to depth 48; scalar values under reviewed measurement keys
are not full content-classified. A malicious peripheral can still mislabel a
reviewed measurement field. Manually review exported files before sharing.

Configuration applies to CLI exports AND TUI sessions. Default TUI detail argv is
therefore hidden as well. Strict mode can make local process inspection less
convenient; choose deliberately. Numeric identity fields remain for stable metric
association, so strict exports can still be correlated with other local data.

Diagnostics contain counts/durations only. They are accumulated in memory with a
2048-observation bound per timing series; disk export is opt-in and uses private
permissions plus exclusive creation (no overwrite/symlink-follow to a file).
No diagnostics are exported on a failed or forcibly terminated TUI; an empty
reserved file can remain and should be removed before retrying.

Synthetic fixture `crates/bmtop-core/tests/fixtures/profiler-synthetic.json` is not
an actual device report. Privacy tests use this fixture, not user identifiers.
