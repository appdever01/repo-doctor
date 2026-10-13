# Startup fixture

This synthetic server uses Node.js built-ins and needs no dependency installation. Its startup condition uses loopback port 3001; choose another port in both files if that port is occupied.

From the repository root, run:

```sh
repo-doctor check examples/startup-api --startup --strict
```

Repo Doctor checks setup, starts the server, observes TCP readiness and stops its process group. A normal check leaves the configured startup condition skipped. No HTTP payload is sent.

To reproduce an application failure after its declared setup succeeds, copy this fixture to a temporary directory and add `require("repo-doctor-fixture-absent-module");` at the beginning of the copied server. Setup observations still pass; `--startup` detects the nonzero exit. Remove that synthetic missing import and rerun to verify recovery. The fixture contains no credentials and never approves dependency scripts.
