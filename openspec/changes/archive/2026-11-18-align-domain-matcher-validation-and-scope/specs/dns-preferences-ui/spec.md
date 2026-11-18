# Spec Delta

## ADDED Requirements

### Requirement: DNS rule dialog and list validate every matcher type
The DNS rule dialog SHALL validate GeoSite, Domain Suffix, Domain Keyword, and Domain Full values inline before save, regardless of whether the DNS master toggle is enabled, rejecting invalid values and enabling save for valid values including `*.example.com` and `ru` on Domain Suffix. Stored DNS rules with invalid values of any matcher type SHALL load unchanged, and their rows SHALL apply error styling and show the validation message as their subtitle.

#### Scenario: Wildcard suffix accepted and wildcard full-domain rejected in DNS dialog
- **WHEN** the user enters `*.example.com` in the DNS rule dialog
- **THEN** selecting Domain Suffix SHALL pass inline validation, and switching to Domain Full SHALL show an inline error and disable save even while DNS is disabled

#### Scenario: Stored invalid DNS suffix, GeoSite, or full-domain row badged on load
- **WHEN** stored settings contain custom DNS rules with Domain Suffix `.ru`, GeoSite `GOOGLE`, or Domain Full `*.example.com`
- **THEN** settings SHALL load unchanged and each invalid rule row SHALL carry error styling and show its validation error as the subtitle
