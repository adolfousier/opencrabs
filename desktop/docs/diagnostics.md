# Desktop diagnostics and failure handling

The Usage panel exposes a **Diagnostics** card. It invokes `get_diagnostics` and shows a read-only local snapshot containing:

- desktop package version;
- presence of the OpenCrabs config and database files;
- today’s OpenCrabs log path;
- a bounded tail of today’s log; and
- operational notes when the log is unavailable.

## Safety boundary

The diagnostics command does not export arbitrary files and does not read config or database contents. Log previews are capped at 128 KiB and 120 lines. Lines containing common credential patterns (`api_key`, `api-key`, `x-api-key`, `Authorization:`, `Bearer `, `token=`, `secret`, or `password`) are omitted before the UI receives them.

## Current limitations

- This surface is a bounded in-app snapshot, not a downloadable crash bundle or arbitrary log/file export.
- Desktop state persists in `desktop-panes.toml`: full pane split tree, focus, pane/session mapping, active route, and selected session. Invalid saved route/session state falls back safely.
- Channel status reports configuration and credential readiness only; it is not a socket or provider availability probe.
- Update installation and voice operations remain explicitly unsupported in the desktop contract. Diagnostics can report their failures, but cannot make those features available.

## Support workflow

1. Reproduce the failure.
2. Open **Usage → Diagnostics** and refresh the snapshot.
3. Record the package version, config/database presence, and the safe log tail.
4. Follow `docs/release-runbook.md` for release/build failures.
5. Do not paste raw `config.toml`, `keys.toml`, database files, or unfiltered log files into tickets.

This is deliberately a support snapshot, not a general filesystem export surface.
