# Contributing

Read START-HERE.zh-CN.md and the module map before changing native sampling.
Run `scripts/verify.sh` on a Mac. Existing locked crates are retained; no new
third-party runtime packages were introduced.

Keep bmtop-core independent of Apple APIs, SoC ownership exclusive to one
sampler, UI rendering free of subprocesses, queues and history bounded, and
unknown metrics distinguishable from zero. Do not add sudo, network uploads,
daemons or Screen Recording prompts to a default path. Preserve PID/start-time
verification before process signals. Avoid copying competitor code with
incompatible licenses; review new dependency licenses and update notices.

Bug reports should include the exact source/release version, chip family and OS
build, reproduction, interval, privacy setting and reviewed doctor JSON. Use
synthetic fixtures for parsing/privacy regressions; label hardware captures and
remove identifiers before committing. Never use --privacy off output in public
issues. Tests must not read developer dotfiles or mutate global environment.

Repo administrators must separately enable public issue/discussion and private
security-reporting channels. Templates do not change GitHub settings.
