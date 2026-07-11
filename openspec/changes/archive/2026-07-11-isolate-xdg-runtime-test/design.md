## Context

`AppPaths::for_profile` (crates/core/src/persistence/mod.rs) resolves `runtime_dir` and `state_dir` by reading `std::env::var("XDG_RUNTIME_DIR")` / `std::env::var("XDG_STATE_HOME")` directly and, when present, deriving the directory via `directories::BaseDirs`. To exercise the unset-fallback branch, `test_runtime_dir_fallback` and `test_state_dir_fallback` call `unsafe { env::remove_var(..) }` / `set_var(..)`. Under `cargo test`'s default parallelism this races every other test that resolves paths (`test_new`, `test_new_and_new_dev_delegation`, `test_with_overrides_partial`, `test_profile_isolation_verification` — all call `for_profile`/`new()`), and a panic between remove and restore leaves the process env dirty. A `profile::Env`/`StdEnv` seam already exists and is used by `AppProfile::resolve(&dyn Env)` and `PathOverrides::resolve(&dyn Env)`; `for_profile` simply never adopted it.

## Goals / Non-Goals

- Goal: XDG-unset fallback is testable with zero process-global env mutation.
- Goal: reuse the established `Env` seam rather than invent a parallel mechanism.
- Non-goal: changing resolved paths for real callers, or the `new()`/`new_dev()`/`for_profile` public signatures.
- Non-goal: adding a test-serialization dependency (`serial_test`, `temp-env`) — the seam removes the need.

## Decisions

- Add an env-injectable resolver, e.g. `for_profile_with_env(profile, env: &dyn Env)`, and have `for_profile` delegate with `&StdEnv`. Public callers are unchanged.
- Compute the runtime/state directory from the injected value directly (`PathBuf::from(value).join(qualifier)`) instead of `BaseDirs::runtime_dir()`/`state_dir()`. This is required for correctness under a mock: `directories::BaseDirs` reads the real process env internally, so a `MockEnv` value would be ignored and the mock would desync from what's resolved. The existing `env::var(..).is_ok()` guard plus `BaseDirs` join is replaced by a single read of the injected source. Verify the resolved production path is byte-identical to today's (`$XDG_RUNTIME_DIR/<qualifier>`), since `BaseDirs::runtime_dir()` returns exactly `$XDG_RUNTIME_DIR` with no extra segments.
- Tests use the existing `MockEnv` pattern (present in `profile.rs` tests); `test_runtime_dir_fallback`/`test_state_dir_fallback` drop the `unsafe` env calls entirely and assert against a mock with and without the keys.

## Risks / Trade-offs

- [Replacing `BaseDirs` for runtime/state could shift the production path] → assert equality against the pre-change resolution in a test that supplies the real `XDG_RUNTIME_DIR` value via the seam; `BaseDirs::runtime_dir()` is documented to return `$XDG_RUNTIME_DIR` verbatim, so `PathBuf::from(value)` matches.
- [Other tests still call `for_profile` reading real env] → acceptable: they only resolve (never `ensure_dirs`) and no longer race a mutating sibling once the two offenders stop mutating global env.

## Migration Plan

Single PR, test + internal-resolver only. Rollback = revert.

## Open Questions

None.
