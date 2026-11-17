# Spec Delta

## ADDED Requirements

### Requirement: Listen address apply, distinct ports, Real Delay preset sync, and thread-safe locale init
The Network preferences page SHALL enable the apply button on the listen-address entry row, reject `socks_port == http_port` with inline validation, initialize the Real Delay URL preset selector to match the loaded URL while clearing `"error"` styling on valid edits, and initialize `LC_MESSAGES` once in `main` before spawning threads without calling MT-unsafe `setlocale` at runtime.

#### Scenario: Listen address row exposes apply action
- **WHEN** the user edits the listen address in Network preferences
- **THEN** `adw::EntryRow` SHALL show its apply button (`show_apply_button(true)`) so pressing Enter or clicking Apply validates and saves the address

#### Scenario: Colliding SOCKS and HTTP ports are rejected inline
- **WHEN** the user sets the HTTP proxy port equal to the SOCKS5 proxy port
- **THEN** the page SHALL mark the port row invalid with an inline error and SHALL NOT persist the colliding port
