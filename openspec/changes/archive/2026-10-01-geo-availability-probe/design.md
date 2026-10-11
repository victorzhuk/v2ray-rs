## Context

- The verdict method was validated live during the exit-IP diagnosis: `GET https://notebooklm.google.com/` following redirects lands on `https://notebook.google/?location=unsupported` from a rejected exit and on `accounts.google.com/…signin…` from an accepted one; no authentication is required for the gate to answer. Gemini's refusal is a body marker on an otherwise-200 landing page.
- The probe infrastructure this builds on (ephemeral backend, one config, N outbounds, per-node loopback inbounds, single-session concurrency rule) is specified by `real-delay-latency-test` and extended by show-node-egress. This change adds no new backend interaction.
- Verdicts are unauthenticated gates only. Post-login region checks (account country) are invisible without the user's Google session and are out of scope.

## Decisions

- **Verdict derivation is a pure function** over (final URL, status, body excerpt) against a check definition `{blocked_final_url_markers, blocked_body_markers}`; anything not matching a blocked marker is `available`, and transport failure (timeout, TLS, no connection) is `unknown`. Pure and table-driven keeps the gate logic unit-testable without a backend and makes adding services a data change.
- **Redirect following is capped at 5 hops with a 2 MiB body cap** and the same timeout as the Real Delay probe; a conventional browser `User-Agent` is sent because Google serves different pages to unidentified clients. No cookies are sent or stored.
- **One request per service per node**, issued after the node's egress fetch in the same session, so a full pass over N nodes × M services costs N×(delay+egress+M) requests through one ephemeral backend.
- **Persistence rides the latency snapshot** as `verdicts: {service_id: {result, checked_at}}` per node; `unknown` and stale entries are representable and never render as a badge without a timestamp.
- **The results view is a dialog from the subscriptions page** ("Availability" action on a node selection), not per-row suffixes: verdicts are per service per node, which does not fit the two-suffix row budget specified in `ui-lists`. A `blocked` badge shows the node's egress sample inline when present, turning the explanation ("this exit, not the protocol") into something visible.
- **Out of scope:** user-defined service checks, authenticated session probes, and scheduled background re-checks.

## Risks

- Service gates are adversarial and change without notice; a stale marker flips verdicts to `available` incorrectly only if Google removes a refusal marker, and to `blocked` only if it adds one — the checked-at timestamp keeps the user able to judge freshness.
- Body-marker matching on a foreign page is locale-sensitive; markers are matched against the served body as-is, and the built-ins use the English refusal strings Google serves by default.
