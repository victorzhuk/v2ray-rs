## ADDED Requirements

### Requirement: Generated configs apply the backend log settings
The generated connection config SHALL set the backend's log verbosity from the backend log level setting. For v2ray and xray, `log.loglevel` SHALL be the level (`error`, `warning`, `info`, `debug`), and `log.access` SHALL be `"none"` unless the connection log setting is on, in which case `log.access` SHALL be omitted. For sing-box, `log.level` SHALL be `error`, `warn`, `info`, or `debug` for the respective level, and the connection log setting SHALL have no effect on the generated config. Probe configs used for latency testing SHALL keep their fixed log settings.

#### Scenario: Defaults silence xray access lines
- **WHEN** a config is generated for xray with default settings
- **THEN** the `log` object SHALL be `{"loglevel": "warning", "access": "none"}`

#### Scenario: Connection log on for v2ray
- **WHEN** a config is generated for v2ray with level `info` and the connection log on
- **THEN** `log.loglevel` SHALL be `"info"` and `log.access` SHALL be absent

#### Scenario: sing-box level mapping
- **WHEN** a config is generated for sing-box with level `warning` and the connection log on
- **THEN** the `log` object SHALL be `{"level": "warn"}`
