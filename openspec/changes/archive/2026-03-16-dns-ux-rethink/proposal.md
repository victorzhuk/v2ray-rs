# Proposal: DNS UX Rethink

## Why
The DNS preferences page is still expert-heavy. The default experience should emphasize the standard `remote` and `domestic` roles, but the app must keep the existing `use_custom_rules` semantics and replace-only provider presets so advanced users do not lose control or run into ambiguous merge behavior.

## What Changes
- **Simplified default surface**: Show `remote` and `domestic` DNS rows as the primary UI and move the full server list behind Advanced.
- **Keep custom-rules mode**: Preserve `use_custom_rules`, but move the toggle and editable custom-rule list into Advanced.
- **Replace-only presets**: Keep provider presets replace-only with the existing confirmation flow; preset application still replaces the server list and strategy without introducing merge semantics.
- **Clear validation scope**: Extend `DnsConfig::validate()` for FakeIP CIDR checks and use it for real-time validation of duplicate tags, invalid rule targets, invalid client subnet, and invalid FakeIP ranges.

## Capabilities

### Modified Capabilities
- `dns-provider-presets`
- `dns-preferences-ui`
- `dns-configuration`
