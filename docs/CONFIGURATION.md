# Configuration v1

Start with `config.example.toml`. Recognized preferences: config_version (1),
interval, language (zh/en), privacy (strict/default/off), default_mode (the nine
snake_case modes), and process.sort/tree/hide_idle/show_full_path.

Precedence: CLI arguments > BMTOP_* environment > file > built-in defaults.
`BMTOP_INTERVAL`, `BMTOP_LANG`, `BMTOP_PRIVACY` map to their flags. Explicit
`--config PATH` overrides BMTOP_CONFIG. `--no-config` ignores all config files
including BMTOP_CONFIG; it does NOT disable interval/language/privacy variables.
Explicit `--no-config` and `--config` conflict rather than silently choosing.

Default search order (first existing path only): absolute
`$XDG_CONFIG_HOME/bmtop/config.toml`, `$HOME/.config/bmtop/config.toml`, then on
macOS `$HOME/Library/Application Support/bmtop/config.toml`. No files are created
or rewritten on launch, quit or key presses. Config files cannot enable FPS,
privileged enhanced collection, or process signals.

Unknown keys, future versions, malformed TOML, non-UTF-8 input and files larger
than 64 KiB are errors. A file is validated before overrides; an invalid file is
not silently repaired by CLI overrides. Use `--no-config` for recovery.
`--print-config` prints resolved TOML and exits without native collection.
`--schema-version` and completion bypass config loading.

Help language is chosen before clap parses/exits, from --lang/BMTOP_LANG/locale,
not from the config file. Runtime language uses the full precedence chain.
The process preferences apply to the TUI; `ps --sort` remains explicit CLI state.
Runtime key changes are session-only; there is not yet a save-preferences command.
