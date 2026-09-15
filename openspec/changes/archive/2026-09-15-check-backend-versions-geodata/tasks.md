## 1. Version gate

- [x] 1.1 Generalize `xray_version_triple` to a backend version probe; add `SINGBOX_MIN_VERSION` (1, 13, 0) gate for every sing-box start with `ProcessError::BackendTooOld { backend, installed, required }` (renames `TunBackendTooOld { installed }`; callers and tests migrate, `is_host_level()` stays true for the variant); unreadable version (xray TUN or sing-box) pushes `warning: could not read <backend> version; minimum-version check skipped` to the log stream; stub tests: `sing-box 1.12.4` → error before `check` runs, `sing-box 1.13.0` → proceeds, empty output → warning line and proceeds

## 2. Geodata preflight

- [x] 2.1 Pure `missing_geodata(backend, rules, subscriptions, dns, geoip_exists, geosite_exists) -> bool` (global enabled rules, active imported-profile rules, custom DNS rules); unit tests: xray + GeoSite rule + no file → true, sing-box → false, no geo rules → false, imported profile geo rule → true
- [ ] 2.2 `start_connection` checks `missing_geodata` before spawning and shows a toast with a "Download geodata" button that runs `download_geodata` off the GTK thread and toasts the outcome; manual check in the dev profile with `geosite.dat` removed

## 3. Verification

- [x] 3.1 `timeout 5m cargo test -p v2ray-rs-process -p v2ray-rs-ui -- --test-threads=4` green
