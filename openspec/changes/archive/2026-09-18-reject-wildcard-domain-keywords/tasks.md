## 1. Core validation and persistence

- [x] 1.1 Reject `*` in domain keyword validation; add core tests for wildcard rejection, plain keyword acceptance, and validated routing mutation rejection.
- [x] 1.2 Add a persistence regression test proving stored wildcard keyword rules load unchanged and survive a save/load round trip.

## 2. Preferences UI

- [x] 2.1 Mark stored invalid routing keyword rows with error styling and the validation error as their subtitle; unit-test the pure row validation helper.
- [x] 2.2 Validate DNS Domain Keyword input before save and mark stored invalid DNS keyword rows with error styling and the validation error subtitle; unit-test the validation helper.

## 3. Verification

- [x] 3.1 Run `timeout 5m cargo test -p v2ray-rs-core -- --test-threads=4` and targeted UI tests.
- [x] 3.2 Run `timeout 10m cargo test --workspace -- --test-threads=4`.
