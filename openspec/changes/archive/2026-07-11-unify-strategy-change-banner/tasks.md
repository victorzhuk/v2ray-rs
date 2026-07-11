## 1. Snapshot tracking

- [x] 1.1 Add `auto_resolve_strategy` and `use_real_delay_for_lowest_latency` to `RuntimeConfigSnapshot`; populate at snapshot creation
- [x] 1.2 Include both fields in `diverges_from`; extend `restore_settings` to restore them
- [x] 1.3 Unit tests: divergence detected on strategy change and on the Real Delay toggle; restore round-trips

## 2. Remove the special case

- [x] 2.1 Delete the `strategy_changed` immediate-disconnect block in `FlushSettings`; verify the generic `check_restart_required` path raises the banner while connected
- [x] 2.2 Verify disconnected-path behavior unchanged (silent apply, no banner)

## 3. Verification

- [ ] 3.1 `cargo test --workspace` green; `cargo clippy` clean; manual run: strategy change while connected shows banner, connection stays up, Apply & Restart reconnects with the new strategy
- [x] 3.2 CHANGELOG entry noting the consent-gated behavior change
