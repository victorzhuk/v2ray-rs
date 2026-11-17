# network-preferences-ui Specification

## Purpose
Defines how the Network preferences page tells the user which backends honor each connection setting, so a setting that the selected backend ignores is not mistaken for a working one.

## Requirements

### Requirement: Network settings state backend applicability
The Network preferences page SHALL state, on each setting that only some backends honor, which backends apply it. The idle connection timeout SHALL be noted as applying to v2ray and xray only. The WebSocket ping interval SHALL be noted as applying to xray only.

#### Scenario: Idle timeout note
- **WHEN** the user opens the Network page
- **THEN** the idle connection timeout row SHALL state that it applies to v2ray and xray and that sing-box ignores it

#### Scenario: WebSocket ping note
- **WHEN** the user opens the Network page
- **THEN** the WebSocket ping interval row SHALL state that it applies to xray only

### Requirement: Listen address apply, distinct ports, Real Delay preset sync, and thread-safe locale init
The Network preferences page SHALL enable the apply button on the listen-address entry row, reject `socks_port == http_port` with inline validation, initialize the Real Delay URL preset selector to match the loaded URL while clearing `"error"` styling on valid edits, and initialize `LC_MESSAGES` once in `main` before spawning threads without calling MT-unsafe `setlocale` at runtime.

#### Scenario: Listen address row exposes apply action
- **WHEN** the user edits the listen address in Network preferences
- **THEN** `adw::EntryRow` SHALL show its apply button (`show_apply_button(true)`) so pressing Enter or clicking Apply validates and saves the address

#### Scenario: Colliding SOCKS and HTTP ports are rejected inline
- **WHEN** the user sets the HTTP proxy port equal to the SOCKS5 proxy port
- **THEN** the page SHALL mark the port row invalid with an inline error and SHALL NOT persist the colliding port
