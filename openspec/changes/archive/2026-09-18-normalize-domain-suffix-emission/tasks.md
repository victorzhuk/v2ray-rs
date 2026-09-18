## 1. Suffix normalization

- [x] 1.1 Add a shared emission helper that removes one leading `*.` and unit-test wildcard and plain names.

## 2. Backend output

- [x] 2.1 Apply normalized suffix emission to xray/v2ray routing domain rules; update routing tests to assert no emitted suffix value contains `*`.
- [x] 2.2 Apply normalized `domain:` emission to xray/v2ray DNS rules, derived DNS domains, and all TUN exclusion routing and DNS paths; extend focused config tests.
- [x] 2.3 Apply normalized suffix emission to sing-box routing, DNS rules, derived DNS domains, and TUN exclusion paths; extend focused config tests.

## 3. Verification

- [x] 3.1 Run `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` and keep generator tests green.
- [x] 3.2 Run `timeout 10m cargo test --workspace -- --test-threads=4`; when xray is installed, validate generated xray suffix syntax with `xray run -test`.
