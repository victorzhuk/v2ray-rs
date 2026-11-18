# Spec Delta

## ADDED Requirements

### Requirement: Subscription HTTP fetches block private initial URLs and DNS answers
Subscription HTTP fetches SHALL validate the initial request URL and every resolved DNS socket address against the same public-target policy applied to redirects, rejecting loopback, link-local, RFC 1918 private, RFC 6598 shared, RFC 2544 benchmarking, unique-local, unspecified, broadcast, IPv4-mapped private IPv6 addresses, and `localhost` hostnames before connecting.

#### Scenario: Direct private or metadata subscription URL is rejected before connect
- **WHEN** a user or `v2raytun://import/` link supplies `http://127.0.0.1:8080/sub`, `http://169.254.169.254/latest/meta-data/`, or `https://localhost/sub`
- **THEN** the subscription fetch SHALL fail immediately as a non-transient error without opening a socket or retrying

#### Scenario: Public hostname resolving to a private IP is rejected
- **WHEN** a subscription URL uses a non-`localhost` domain name whose DNS lookup returns `127.0.0.1`, `10.0.0.1`, `169.254.169.254`, or `::1`
- **THEN** the subscription client SHALL refuse the resolved address and fail the fetch before establishing a TCP or TLS connection

### Requirement: Subscription imports bound parsed node count
Subscription imports and refreshes across URI-list and JSON configuration-bundle sources SHALL enforce an upper bound of 4,096 parsed proxy nodes per subscription, failing oversized node lists without replacing existing subscription data or rendering unbounded widget trees.

#### Scenario: URI subscription exceeding maximum node count fails cleanly
- **WHEN** a fetched or local subscription decodes to more than 4,096 valid proxy nodes within the 10 MiB payload limit
- **THEN** the import SHALL return a descriptive error naming the node limit without persisting the oversized subscription

#### Scenario: Refresh exceeding node cap preserves previous subscription nodes
- **WHEN** refreshing an existing subscription produces more than 4,096 parsed nodes
- **THEN** the refresh SHALL fail and the previously stored nodes for that subscription SHALL remain intact
