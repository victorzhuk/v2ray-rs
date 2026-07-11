## Why

`make test` can fail nondeterministically because persistence tests mutate process-global `XDG_RUNTIME_DIR` and `XDG_STATE_HOME` while other tests resolve paths concurrently. This blocks reliable verification of unrelated changes.

## What Changes

- Thread the existing `Env` seam (`profile::Env`/`StdEnv`, already used by `AppProfile::resolve` and `PathOverrides::resolve`) through XDG runtime/state directory resolution so it reads through an injectable source instead of `std::env::var` directly.
- Test XDG-unset fallback behavior with a `MockEnv` instead of mutating process-global `XDG_RUNTIME_DIR`/`XDG_STATE_HOME`, removing the `unsafe { env::set_var/remove_var }` calls that race with concurrent tests.
- Preserve runtime path behavior for application callers (they resolve through `StdEnv`, unchanged).

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `runtime-profiles`: adds a requirement making the XDG-unset fallback (`XDG_RUNTIME_DIR`/`XDG_STATE_HOME` absent → `data_dir/runtime`|`data_dir/state`) normative and resolvable through an injected environment source. This documents behavior the tests already assert but the spec did not state, and is what makes deterministic testing possible.

## Impact

- `crates/core/src/persistence/mod.rs` — `for_profile` delegates to an env-injectable resolver; runtime/state dirs computed from the injected value rather than `directories::BaseDirs` (which reads real process env and would desync from a mock).
- Persistence unit tests — the two XDG fallback tests stop mutating global env; they use `MockEnv`.
- No public application API change (`new()`/`new_dev()`/`for_profile` keep their signatures, delegating to `StdEnv`), no configuration, dependency, or migration changes.
