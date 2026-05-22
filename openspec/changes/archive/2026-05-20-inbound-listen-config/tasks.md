# Tasks: inbound-listen-config

## 1. Settings model

- [x] 1.1 Add `listen_address: String` field to `AppSettings` in `crates/core/src/models/settings.rs` with `#[serde(default = "default_listen_address")]`.
- [x] 1.2 Add `default_listen_address() -> String` returning `"127.0.0.1".to_string()`.
- [x] 1.3 Initialise the field in `AppSettings::default()`.
- [x] 1.4 Add `AppSettings::validate_listen_address(&str) -> Result<(), ValidationError>` that parses with `std::net::IpAddr::from_str` and rejects hostnames and empty strings.
- [x] 1.5 Extend `ValidationError` with an `InvalidListenAddress(String)` variant if no suitable variant exists.

## 2. Persistence regression coverage

- [x] 2.1 Add a unit test loading a legacy `settings.toml` (without `listen_address`) and assert the field deserialises to `"127.0.0.1"`.
- [x] 2.2 Add a round-trip test: set `listen_address = "0.0.0.0"`, serialise, deserialise, assert equality.
- [x] 2.3 Add a validation test covering `"127.0.0.1"`, `"0.0.0.0"`, `"::"`, `"::1"`, `"192.168.1.10"` (all OK) and `""`, `"localhost"`, `"not-an-ip"` (all errors).

## 3. v2ray/xray generator

- [x] 3.1 Change `build_inbounds` in `crates/core/src/config/v2ray.rs` to read `settings.listen_address` for both the SOCKS and HTTP inbounds instead of the literal `"127.0.0.1"`.
- [x] 3.2 Add a test `test_inbound_listen_address_from_settings` asserting both inbounds reflect a non-default value (e.g. `"0.0.0.0"`).
- [x] 3.3 Add a regression test `test_socks_inbound_udp_enabled` asserting `config["inbounds"][0]["settings"]["udp"]` equals `true`.
- [x] 3.4 Add the same listen-address test under the xray generator's test module to confirm the inherited behaviour.

## 4. sing-box generator

- [x] 4.1 Change `build_inbounds` in `crates/core/src/config/singbox.rs` to read `settings.listen_address` for both the `mixed` and `http` inbounds.
- [x] 4.2 Add a test `test_singbox_inbound_listen_address_from_settings` asserting both inbounds reflect a non-default value.
- [x] 4.3 Add a regression test `test_singbox_mixed_inbound_udp_enabled` asserting `config["inbounds"][0]["type"] == "mixed"` and that `config["inbounds"][0].get("udp_disabled")` is not `true`.

## 5. Defensive validation in writer

- [x] 5.1 In `ConfigWriter::write_config` (or the wrapper before calling `ConfigGenerator::generate`), validate `settings.listen_address`; on failure, log a `tracing::warn` and substitute `"127.0.0.1"` for the generator call.
- [x] 5.2 Add a unit test that confirms an invalid `listen_address` produces a config with `"127.0.0.1"` and emits no error.

## 6. UI

- [x] 6.1 Add a "Listen address" entry row to the Settings page in `crates/ui/src/settings.rs`, bound to `AppSettings::listen_address`.
- [x] 6.2 On commit, call `AppSettings::validate_listen_address`; surface errors via the existing toast mechanism without persisting.
- [x] 6.3 When the saved value is non-loopback (anything other than `127.0.0.1`, `::1`), emit a one-shot warning toast: "Proxy now reachable from other hosts on this network."
- [x] 6.4 Update any existing settings-related snapshot tests / fixtures that hard-code `AppSettings` to include the new field.

## 7. Documentation

- [x] 7.1 Update `CLAUDE.md` `crates/core` section to mention `listen_address` in the `AppSettings` summary.
- [x] 7.2 Update `CHANGELOG.md` with a `Changed` entry: "Inbound listen address is now configurable (default `127.0.0.1`)." and an `Added` entry for the new setting.
- [x] 7.3 Verify `cargo test --workspace` passes after all changes.
