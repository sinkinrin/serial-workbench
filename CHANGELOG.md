# Changelog

## Unreleased — Shared Serial Workbench

- Show command previews and expandable planned steps / actual TX-RX details; persist task records across daemon restarts and mark unfinished work interrupted without replay.
- Share saved scripts across GUI, CLI and MCP with revision checks, immutable execution snapshots and one-time migration from browser storage. Add five MCP script tools and CLI `--details`.

- Add named quick scripts: save one or multiple text commands, choose line endings / intervals / optional response waits, and run through the shared macro queue.

- Add editable keyword/color rules with ordering, enable toggles, case matching, live preview and browser-local persistence.

- Compact the workbench into a viewport-sized layout with tool tabs and log focus mode; reassemble serial log lines and highlight device modules, counters, timing, warnings and errors without changing raw capture.

- Add an independent serial owner with non-consuming cursor reads, rotating disk logs and a shared task queue.
- Add an embedded Chinese browser UI, xterm.js terminal, JSON CLI and thin MCP adapter.
- Preserve legacy direct serial tools and background recording changes.
- Add isolated simulator integration tests and verify read-only command batches on a real COM6 CH340 device. USB hotplug and electrical behavior remain unverified.

## Unreleased

### Added

- Added MCP background serial logging with `logging_start`, `logging_status`,
  `logging_tail`, and `logging_stop`.
- Added size-based log rotation, bounded in-memory tails, recording statistics,
  and automatic flush-on-close/shutdown behavior.
- Added bounded capture windows for CLI `read`, CLI `write --read`, and MCP
  `read` using `duration_ms`, `start_trigger`, `initial_timeout_ms`, and
  `idle_timeout_ms`.
- Added capture response metadata: `completion_reason`, `waited_ms`,
  `elapsed_ms`, and per-chunk byte offsets.

### Changed

- Background recording now owns a connection's read direction. Ordinary MCP
  reads and real-hardware macro runs are rejected until recording stops, while
  writes remain available.
- Preserved existing single-read behavior when `duration_ms` is not supplied.

## 0.3.0 - 2026-06-24

### Added

- Added v0.3 JSON Macro DSL packs with strict validation for macros and assemblies.
- Added CLI macro commands: `macro validate`, `macro list`, `macro plan`, and `macro run`.
- Added macro executor support for `send`, `delay`, `expect contains`, and `expect equals`.
- Added explicit simulation mode for no-hardware macro execution tests and CLI smoke checks.
- Added MCP macro tools: `macro_load`, `macro_list`, `macro_unload`, `macro_plan`, `macro_run`, and `macro_run_inline`.
- Added runtime-only MCP macro registry. Loaded packs are cleared on server restart.
- Added `examples/macros/ping.json` as a minimal valid macro pack.

### Changed

- Updated package version to `0.3.0`.
- Updated README, README_ZH, and serial-debug skill docs to describe CLI, skill, and MCP macro usage paths.

### Not Included

- No independent Quick API. Quick-style operations should be represented as macros.
- No persistent macro library managed by the server.
- No RTS/DTR macro steps and no general scripting features.

## 0.2.0 - 2026-06-24

### Added

- Added explicit `serve` mode for the MCP stdio server while keeping no-subcommand startup compatible.
- Added one-shot CLI commands: `list-ports`, `probe`, `write`, `read`, and `set-control-lines`.
- Added JSON output for CLI automation and agent skill workflows.
- Added `generate-config`, `validate-config`, and `show-config` subcommands alongside legacy flags.
- Added `skills/serial-debug`, a Codex and Claude Code compatible skill that uses the CLI first and MCP when configured.

### Changed

- Updated Rust MSRV documentation and package metadata to Rust 1.74.
- Updated locked dependency versions away from yanked `serialport 4.7.2` and `slab 0.4.10` after local install smoke testing.
- Updated user-facing docs to distinguish MCP, CLI, skill, and manual hardware validation boundaries.
- Refactored serial/session test modules and connection parameters to satisfy release clippy gates.

### Fixed

- Fixed CLI logging so one-shot commands keep stdout data-only by default.
- Fixed clippy warnings promoted by `-D warnings`.
- Fixed rustdoc warnings in the session documentation.

## 0.1.0

- Initial MCP stdio server with serial port discovery, open, write, read, close, and RTS/DTR control-line support.
