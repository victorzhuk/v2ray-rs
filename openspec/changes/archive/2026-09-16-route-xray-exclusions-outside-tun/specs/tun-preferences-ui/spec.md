## ADDED Requirements

### Requirement: Exclusion lists describe their effect per backend
The excluded-routes and excluded-domains groups SHALL describe what each list does on the active backend. Excluded routes SHALL be described as destinations routed outside the tunnel on both backends. For xray, excluded domains SHALL be described as traffic that still enters the tunnel and is sent directly by xray after it recognizes the domain, so they do not keep traffic off the TUN device; for sing-box, the description SHALL describe domain suffixes that bypass the tunnel. Both groups SHALL state that changes apply on the next connect.

#### Scenario: xray domain exclusion wording
- **WHEN** the active backend is xray and the TUN page is shown
- **THEN** the excluded-domains description SHALL state that matching traffic still passes through the tunnel and is sent directly by xray

#### Scenario: Route exclusion wording
- **WHEN** the active backend is xray or sing-box
- **THEN** the excluded-routes description SHALL state that the CIDRs are routed outside the tunnel and that changes apply on the next connect
