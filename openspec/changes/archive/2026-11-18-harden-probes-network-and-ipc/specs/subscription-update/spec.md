# Spec Delta

## MODIFIED Requirements

### Requirement: Subscription HTTP redirects block private and local targets
Subscription HTTP fetches SHALL evaluate each redirect hop and reject redirects to loopback, link-local, RFC 1918 private, RFC 6598 shared address space (`100.64.0.0/10`), RFC 2544 benchmarking (`198.18.0.0/15`), unique-local, unspecified, or broadcast IP literals (including IPv4-mapped IPv6 forms), or `localhost` hostnames.

#### Scenario: Public subscription redirects to loopback or metadata IP
- **WHEN** fetching an `https://` subscription URL returns a `302` redirect to `http://127.0.0.1:8080/sub` or `http://169.254.169.254/latest/meta-data/`
- **THEN** the fetcher SHALL abort the redirect and return a terminal fetch error without connecting to the redirect target

#### Scenario: Public subscription redirects to CGNAT or benchmarking IP
- **WHEN** fetching an `https://` subscription URL returns a redirect to `https://100.64.0.1/sub`, `https://198.18.0.1/sub`, or `https://[::ffff:100.64.0.1]/sub`
- **THEN** the fetcher SHALL abort the redirect and return a terminal fetch error without connecting to the redirect target

#### Scenario: Public-to-public HTTPS redirect succeeds
- **WHEN** fetching a subscription URL redirects across public hosts within the redirect limit
- **THEN** the fetcher SHALL follow the redirect normally
