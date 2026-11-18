# Spec Delta

## ADDED Requirements

### Requirement: DNS rule matchers validate suffix wildcards and exact hosts
DNS rule validation SHALL accept domain suffixes with an optional single leading `*.` followed by one or more valid ASCII domain labels (including single-label TLD suffixes such as `ru`), while rejecting empty suffixes, leading dots, and interior or repeated `*`. DNS full-domain rules SHALL reject any `*` and require a valid multi-label hostname.

#### Scenario: Wildcard and TLD suffixes accepted in DNS rules
- **WHEN** custom DNS rules contain Domain Suffix `*.example.com` and Domain Suffix `ru`
- **THEN** DNS validation SHALL accept both rules

#### Scenario: Leading-dot suffix and wildcard full-domain rejected in DNS rules
- **WHEN** a custom DNS rule uses Domain Suffix `.ru`, Domain Suffix `*.*.example.com`, or Domain Full `*.example.com`
- **THEN** DNS validation SHALL reject each rule with an error naming the invalid matcher value

### Requirement: Effective DNS domain scopes preserve matcher kind
Domain-scoped Effective DNS entries SHALL format suffix items as `domain:<name>` with one leading `*.` stripped, keyword items as `keyword:<value>`, full-domain items as `full:<host>`, and GeoSite items with the backend's valid GeoSite tag. On sing-box with TUN enabled, non-final split-horizon servers SHALL also include `domain:<name>` items for TUN excluded domains.

#### Scenario: sing-box custom and auto-derived scopes keep matcher prefixes
- **WHEN** the backend is sing-box, DNS is enabled with `remote` as final server and `domestic` scoped by suffix `*.example.ru`, keyword `yandex`, full domain `api.example.ru`, and GeoSite `category-ru`
- **THEN** the `domestic` Effective DNS entry SHALL have scope `geosite-category-ru`, `domain:example.ru`, `keyword:yandex`, and `full:api.example.ru`

#### Scenario: Unrepresentable sing-box GeoSite tag omitted from Effective DNS scope
- **WHEN** the backend is sing-box and a rule scopes `domestic` to negated GeoSite `geolocation-!cn` alongside suffix `cn`
- **THEN** the `domestic` Effective DNS scope SHALL include `domain:cn` and SHALL NOT include `geosite-geolocation-!cn`

#### Scenario: sing-box TUN excluded domains appear in non-final split-horizon scope
- **WHEN** the backend is sing-box, TUN is enabled with excluded domain `*.corp.example.com`, `remote` is the first DNS server, and `domestic` has detour `direct`
- **THEN** the `domestic` Effective DNS scope SHALL include `domain:corp.example.com`
