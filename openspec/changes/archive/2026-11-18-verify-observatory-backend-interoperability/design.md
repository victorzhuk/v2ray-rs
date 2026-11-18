## Decisions

Use local backend fixtures and a loopback HTTP endpoint to make the result independent of public servers. Drive the production probe entry point. Include a reachable node and an unavailable node to expose incorrect result attribution. Use isolated ports and bounded child lifetimes.

## Risks and recovery

Backend version drift can change observatory service behavior. Record binary versions and backend diagnostics on failure. Terminate fixture processes on all exits.
