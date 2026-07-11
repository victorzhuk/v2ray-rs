## 1. Env-injectable resolution

- [x] 1.1 Add `AppPaths::for_profile_with_env(profile, env: &dyn Env)` resolving `runtime_dir`/`state_dir` from `env.get("XDG_RUNTIME_DIR")`/`env.get("XDG_STATE_HOME")`, computing the directory from the injected value joined with the qualifier and falling back to `data_dir/runtime`|`data_dir/state` when absent
- [x] 1.2 Make `for_profile` delegate to `for_profile_with_env(profile, &StdEnv)`; keep `new()`/`new_dev()` unchanged
- [x] 1.3 Drop the direct `std::env::var("XDG_RUNTIME_DIR")`/`XDG_STATE_HOME` reads and the `BaseDirs` runtime/state derivation from the resolver

## 2. Deterministic tests

- [x] 2.1 Rewrite `test_runtime_dir_fallback` and `test_state_dir_fallback` to use `MockEnv` (with and without the key); remove all `unsafe { env::remove_var/set_var }` calls
- [x] 2.2 Add a test asserting the production path (`XDG_RUNTIME_DIR` present) resolves byte-identically to the pre-change `$XDG_RUNTIME_DIR/<qualifier>` value, via the injected seam

## 3. Verification

- [x] 3.1 `cargo test -p v2ray-rs-core` green; run the suite repeatedly (e.g. `cargo test -p v2ray-rs-core -- --test-threads=8`, a few iterations) to confirm the fallback tests no longer flake
- [x] 3.2 `cargo clippy` clean; confirm no remaining `unsafe` env mutation in the persistence tests
