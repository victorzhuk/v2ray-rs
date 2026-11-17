# Spec Delta

## ADDED Requirements

### Requirement: TUN preferences guard rollback re-entrancy and parse Run-with-bypass argv
Programmatic widget rollbacks in TUN preferences SHALL suppress re-entrant signal handlers via a guard flag, `v2ray-rs-bypass` account lookup SHALL run off the GTK main thread, and `Run with bypass` SHALL parse command lines with `glib::shell_parse_argv` and report non-zero child exit statuses to the user.

#### Scenario: Quoted arguments in Run with bypass are preserved
- **WHEN** the user runs `curl -H "Accept: application/json" https://example.com` via `Run with bypass`
- **THEN** `v2ray-rs-run` SHALL receive `["curl", "-H", "Accept: application/json", "https://example.com"]` without splitting inside quotes

#### Scenario: Failed bypass command surfaces error toast
- **WHEN** a command launched via `Run with bypass` exits immediately with a non-zero status or spawn error
- **THEN** the UI SHALL surface an error toast naming the failure
