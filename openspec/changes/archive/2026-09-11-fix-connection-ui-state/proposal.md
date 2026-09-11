# Keep the app's connection state honest about what is running

## Why

A review of the connection state machine on 2026-09-11 found places where the app acts on the wrong node, misses a pending restart, or rolls back its own records:

- **Apply & Restart and automatic reconnect drop a directly chosen node.** Both go through `Connect`, which re-runs the planner. A user who connected to node X directly, then edited a routing rule and pressed Apply & Restart, is reconnected to whatever the strategy picks. Once X reached `Running` the direct-connect flag is cleared, so a later crash give-up also reconnects elsewhere.
- **TUN and timeout edits never raise the restart banner.** The runtime snapshot ignores TUN settings and the connection idle-timeout and WebSocket-heartbeat settings, all of which change the generated config. Turning TUN on while connected does nothing visible — yet the UI reads the edited flag, marks TUN active and stops background latency probes for a session that has no TUN.
- **Preferences rolls back the last successful node.** The dialog copies all settings when it opens and sends the whole copy on every change; saving it overwrites a last-success record written after the dialog opened, so the Last Successful strategy reconnects to the previous node.
- **A manual Connect leaves a pending automatic reconnect armed.** The old timer still matches and fires `Connect` immediately after the user's own attempt, spending the automatic-reconnect budget on the user's retries.
- **Log lines from a replaced connection land in the new session's view.**

## What Changes

- Apply & Restart and automatic reconnect of a session started by a direct connect reconnect to that node as the sole candidate; if it is no longer present or enabled, the user is told and the configured strategy plans the reconnect.
- The runtime snapshot includes the TUN settings and the idle-timeout and heartbeat settings, so editing them while connected raises the restart banner; UI state derived from TUN (probe suppression, TUN indicators) follows the running session's snapshot.
- Saving settings from Preferences never changes the last-success record.
- A user-initiated Connect or Disconnect cancels any pending automatic reconnect.
- Log lines are tagged with their connection and lines from a superseded connection are dropped.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `process-lifecycle`: "Capture launched runtime snapshot" covers TUN and timeout settings; "Apply pending runtime changes by restart" keeps a directly chosen node and raises the banner for TUN edits.
- `connection-auto-resolve`: "Direct connection to a chosen node" extends to automatic reconnect; new requirements for last-success ownership and automatic reconnect yielding to the user.

## Impact

- `crates/core/src/runtime_snapshot.rs` — new fields and comparisons.
- `crates/ui/src/app.rs` — direct-session target retained, restart/auto-reconnect via that target, reconnect timer cancellation, TUN-active from the snapshot, generation-tagged log lines.
- `crates/ui/src/preferences/mod.rs` / `app.rs` `FlushSettings` — last-success preserved.
- `crates/ui/src/connection.rs` — generation on log messages.
