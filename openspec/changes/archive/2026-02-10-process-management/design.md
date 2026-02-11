# Design: Process Management

## Context

The app launches the selected backend CLI binary with a generated config file, monitors its health, captures output, and handles lifecycle events (start, stop, restart, crash recovery).

## Goals / Non-Goals

**Goals:**
- Async process management with tokio
- Log capture from stdout/stderr with real-time streaming to UI
- Graceful shutdown with SIGTERM → timeout → SIGKILL
- Crash detection and optional auto-restart
- Clean process cleanup on app exit

**Non-Goals:**
- No process sandboxing or privilege escalation
- No remote process management

## Decisions

1. **Async process**: Use `tokio::process::Command` for async process spawning. Capture stdout/stderr via piped streams. Stream lines to a bounded channel for UI consumption.

2. **Signal handling**: Send SIGTERM first, wait 5 seconds, then SIGKILL if still running. Use `nix` crate for signal operations.

3. **State machine**: Model process state as enum: `Stopped → Starting → Running → Stopping → Stopped` (plus `Error` state). Emit state changes as events for UI/tray consumption.

4. **Log buffer**: Keep last 10,000 lines in a circular buffer. UI reads from this buffer. New lines are also emitted as events for live display.

5. **Restart strategy**: On config regeneration, gracefully stop then restart. On crash, wait 2 seconds then restart (max 3 consecutive crash-restarts before entering Error state).

## Risks / Trade-offs

- [Zombie processes] If app crashes without cleanup → Mitigated by storing PID to file and checking/killing on next launch
- [Log memory] Large log buffers consume memory → Bounded circular buffer caps at ~10K lines
