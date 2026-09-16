## 1. Failure reason normalization

- [x] 1.1 Pure `failure_key(reason, label, address, port) -> String` in `crates/ui/src/connection.rs`: strips ANSI SGR escapes, a leading `YYYY/MM/DD HH:MM:SS(.frac)` timestamp, bracketed all-digit tokens, and replaces label/address/port; unit tests: sing-box `[0000]` vs `[0001]` FATAL equal, xray lines with different timestamps equal, TLS errors naming different hosts that are the candidates' own addresses equal, TLS errors naming unrelated hosts differ
- [x] 1.2 Candidate failures keep label and ANSI-stripped reason separately so the loop can compare keys and the summary can print reasons; existing `last_candidate_failure_reports_one_error` still passes

## 2. Early stop

- [x] 2.1 Candidate loop stops when the current failure key equals the previous candidate's key and reports one `Error` with `Connection failed on consecutive nodes with the same error (not node-specific): <reason>`; parked manager handling and Stop semantics unchanged; stub-backend test: 3 candidates exiting with the same FATAL → only 2 launched (count runs), one terminal `Error` with the new wording and no `\x1b`
- [x] 2.2 Stub-backend test: 3 candidates whose failures differ → all 3 launched, existing summary format (ANSI-stripped)

## 3. Verification

- [x] 3.1 `timeout 10m cargo test --workspace -- --test-threads=4` green
- [ ] 3.2 Live: reproduce the sing-box IPv6 FATAL (before `fix-singbox-tun-without-ipv6` lands, or with an invalid TUN setting that fails at startup) → `backend.log` shows exactly two `session` records for the attempt and the toast shows the not-node-specific error
