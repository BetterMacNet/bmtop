# Machine output changes

Schema remains version **2**. New snapshot GPU coverage fields have serde defaults;
legacy snapshots without them still deserialize. Config version 1 and runtime
diagnostics version 1 are separate contracts.

| Command | Table | JSON | JSONL | CSV |
|---|---|---|---|---|
| ps | yes | yes | yes | yes |
| cpu | yes | yes | yes | added |
| memory | yes | yes | yes | added |
| network | yes | yes | yes | yes |
| network --connections | yes | yes | yes | added |
| disk | yes | yes | yes | added |
| gpu | yes | yes | yes | added summary (non-enhanced only) |
| hardware [category] | yes | yes | yes | explicitly rejected |
| sensors | yes | yes | yes | explicitly rejected |
| doctor | yes | yes | fixed single line | explicitly rejected |
| top | interactive only | rejected | rejected | rejected |

Most metric reports use JsonEnvelope. Doctor intentionally retains its legacy
top-level shape instead of an incompatible nested data migration; it now adds
schema_version/kind/version/captured_at plus additive `runtime` and `sources`
compatibility fields. JSON requires a single sample; multiple
samples use JSONL/table/CSV. `--count 0` is a usage error. `--count N` limits watch;
network connections and hardware no longer silently ignore count/watch. Doctor
rejects watch/count. CLI parser errors can be exit 2; semantic usage errors are
64, unavailable capabilities/platform 69, denied authorization 77, other errors 70.

CSV headers are emitted once per stream. Null numeric values become empty cells,
not zero. Strings quote embedded quotes/commas/newlines. This is raw CSV for data
consumers, NOT spreadsheet formula-neutralized CSV; treat imported text as text.
Original process/network columns are retained, so CSV does not identify sample
boundaries for those row sets. Prefer JSONL for replay/event boundaries.

Connection parsing uses `lsof -nP -i -F0pcfnPT`: NUL-delimited fields, LF record
boundaries, per-file `f` flushing. An empty selection is valid; other nonzero
errors are not silently treated as complete data. Available connection coverage
still depends on local permissions and process churn. No packet capture, DNS
lookup, elevated permissions or remote connection is introduced.

## Synthetic examples

The examples below are deliberately synthetic and contain no host identifiers.
Timestamps and values are illustrative; field names and envelope shape are the
contract.

One JSON sample:

```json
{
  "schema_version": 2,
  "kind": "memory",
  "captured_at": "2026-01-01T00:00:00.000Z",
  "source": "available",
  "capabilities": ["memory", "swap"],
  "data": {
    "memory": {
      "total_bytes": 17179869184,
      "used_bytes": 8589934592,
      "swap_used_bytes": 0,
      "swap_total_bytes": 0,
      "pressure_percent": 18.5
    },
    "bandwidth": null
  }
}
```

A JSONL stream contains one complete envelope per line:

```jsonl
{"schema_version":2,"kind":"memory","captured_at":"2026-01-01T00:00:00.000Z","source":"available","capabilities":["memory"],"data":{"memory":{"total_bytes":17179869184,"used_bytes":8589934592},"bandwidth":null}}
{"schema_version":2,"kind":"memory","captured_at":"2026-01-01T00:00:01.000Z","source":"available","capabilities":["memory"],"data":{"memory":{"total_bytes":17179869184,"used_bytes":8623489024},"bandwidth":null}}
```

A flat CSV stream is intended for supported row-oriented commands:

```csv
pid,cpu_percent,resident_bytes,threads,user,name
42,12.5,104857600,8,"example","worker"
43,,20971520,2,"example","helper"
```

Unknown numeric measurements are empty/null, not zero. A readable disk-I/O
source may legitimately report zero rates while idle; an unavailable source is
reported as null. CSV is intentionally rejected for nested hardware, sensors
and doctor reports; use JSON or JSONL for those commands. Common semantic exit
codes are `64` (usage), `69` (unavailable
capability/platform), `70` (other failure), and `77` (permission denied).

`--quick` only accepts single cpu/gpu/sensors samples and uses a real 250ms delta
window; it does not promise the entire command completes in 250ms. Progress is
written only to interactive stderr. No background caching daemon is added.
