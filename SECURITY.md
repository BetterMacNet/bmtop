# Security policy

Automated checks cover Rust formatting, Clippy, workspace tests, doctor smoke
checks, helper tests and native C contracts. They do not constitute a security
audit or certify every macOS release and hardware combination.

Do not post tokens, process argv, serial numbers, UUIDs, network endpoints or
private hostnames in public issues. Use doctor output after manually reviewing
it; use strict privacy for broader diagnostics. --privacy off / --show-sensitive
may expose secrets. See docs/PRIVACY.md for remaining fields and limitations.

Report a vulnerability through the repository's private GitHub security reporting
channel **only if the maintainer has enabled it**. This source patch cannot enable
that setting or guarantee a response SLA. If no private channel is shown, ask the
maintainer for a private reporting route without publishing exploit details or
sensitive data. There is no invented security email address in this policy.

For reports, include version/source hash, affected permission mode, reproduction
using synthetic data, impact and suggested mitigation. We do not require root or
Screen Recording permissions to investigate a report. A maintainer must define
supported release branches and response policy before publishing a supported
release from this patch.
