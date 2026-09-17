## Purpose

Defines how the Network preferences page tells the user which backends honor each connection setting, so a setting that the selected backend ignores is not mistaken for a working one.

## ADDED Requirements

### Requirement: Network settings state backend applicability
The Network preferences page SHALL state, on each setting that only some backends honor, which backends apply it. The idle connection timeout SHALL be noted as applying to v2ray and xray only. The WebSocket ping interval SHALL be noted as applying to xray only.

#### Scenario: Idle timeout note
- **WHEN** the user opens the Network page
- **THEN** the idle connection timeout row SHALL state that it applies to v2ray and xray and that sing-box ignores it

#### Scenario: WebSocket ping note
- **WHEN** the user opens the Network page
- **THEN** the WebSocket ping interval row SHALL state that it applies to xray only
