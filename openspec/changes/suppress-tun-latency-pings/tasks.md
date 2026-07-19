## 1. Scheduled refresh

- [ ] 1.1 Skip the 10-minute background TCP refresh tick while the active connection has TUN enabled
- [ ] 1.2 Unit/behavior test: tick with TUN-active state performs no probes; tick after disconnect resumes

## 2. Manual action

- [ ] 2.1 "Test Latency" (subscriptions page, per-subscription and per-node paths) insensitive during an active TUN session with a hint pointing at "Test Real Delay"
- [ ] 2.2 Re-sensitize on disconnect / TUN-off reconnect

## 3. Verification

- [ ] 3.1 `cargo test --workspace` green
- [ ] 3.2 Manual: connect with TUN on an affected xray (≤ 26.6.22), let two scheduled windows pass and press Test Real Delay — no `panic: Net: Unknown address type.` in logs, no backend restart
