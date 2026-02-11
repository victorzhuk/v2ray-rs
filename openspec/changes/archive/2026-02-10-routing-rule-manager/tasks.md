## 1. Routing Rule Data Model

- [x] 1.1 Define `RuleMatch` enum with variants: `GeoIp(CountryCode)`, `GeoSite(Category)`, `Domain(DomainPattern)`, `IpCidr(IpNet)`
- [x] 1.2 Define `RuleAction` enum with variants: `Proxy`, `Direct`, `Block`
- [x] 1.3 Define `RoutingRule` struct combining `RuleMatch`, `RuleAction`, and ordering metadata
- [x] 1.4 Implement serialization/deserialization for routing rule types
- [x] 1.5 Add routing rules field to user configuration model

## 2. Rule Validation

- [x] 2.1 Implement country code validation against ISO 3166-1 alpha-2 list
- [x] 2.2 Implement IP CIDR parsing and validation
- [x] 2.3 Implement domain pattern validation (wildcard syntax)
- [x] 2.4 Implement GeoSite category validation against known categories

## 3. Rule CRUD Operations

- [x] 3.1 Implement `add_rule` with validation and position insertion
- [x] 3.2 Implement `edit_rule` to update match condition or action
- [x] 3.3 Implement `delete_rule` by index or identifier
- [x] 3.4 Implement `reorder_rule` to move a rule to a new position
- [x] 3.5 Persist rule changes to configuration file

## 4. Predefined Rule Templates

- [x] 4.1 Define preset data structure and built-in presets (RU direct, CN direct, ads block)
- [x] 4.2 Implement `apply_preset` that inserts preset rules into the rule list

## 5. Geodata Download and Storage

- [x] 5.1 Implement geodata download from v2fly GitHub releases (.dat format)
- [x] 5.2 Implement geodata download from SagerNet releases (.db format)
- [x] 5.3 Select correct download source based on active backend type
- [x] 5.4 Store downloaded files in app data directory
- [x] 5.5 Handle download failures gracefully with error reporting

## 6. Geodata Auto-Update

- [x] 6.1 Track last update check timestamp in configuration
- [x] 6.2 Implement periodic update check (default 7 days, configurable)
- [x] 6.3 Compare upstream version against local version
- [x] 6.4 Implement atomic file swap for geodata replacement

## 7. Integration

- [x] 7.1 Wire routing rules into config generation output
- [x] 7.2 Trigger config regeneration on rule add/edit/delete/reorder
- [x] 7.3 Trigger geodata download on first launch when files are missing
