# Spec Delta

## ADDED Requirements

### Requirement: Geodata HTTP redirects enforce public HTTPS upstream hosts
Geodata downloads SHALL follow HTTP redirects only over `https://` within the redirect hop limit, and SHALL reject redirects targeting private, loopback, link-local, CGNAT, benchmarking, unique-local, or `localhost` destinations or hostnames outside the GitHub distribution domains (`github.com` and `*.githubusercontent.com`).

#### Scenario: Geodata redirect to non-GitHub or private host is refused
- **WHEN** a geodata download receives an HTTP redirect to `https://127.0.0.1/geoip.dat`, `https://100.64.0.10/geoip.dat`, or `https://evil.example.com/geoip.dat`
- **THEN** the downloader SHALL refuse the redirect and fail the refresh without writing to the geodata cache

#### Scenario: GitHub release redirect to objects.githubusercontent.com succeeds
- **WHEN** a geodata download from `https://github.com/.../releases/latest/download/geoip.dat` redirects over HTTPS to `https://objects.githubusercontent.com/...`
- **THEN** the downloader SHALL follow the redirect and persist the verified response within the size cap
