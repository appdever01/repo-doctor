# Changelog

## 0.1.1 - 2026-10-13

- Check required direct Node dependencies and development dependencies for installed package manifests without loading package code.
- Report pnpm's recorded ignored and pending dependency builds without running pnpm or approving scripts.
- Keep missing metadata, unsupported installation layouts, production-only development omissions and unsafe paths unverified.
- Bound dependency declarations and pnpm build records to 256, reusing the file-size, combined-size, cancellation and 15-second run limits.

The JSON schema and exit-code contract remain unchanged. Optional dependencies and peer dependencies are excluded. These checks do not establish dependency versions, transitive dependency integrity, native build outputs or application health. Version 0.1.0 remains available unchanged.

## 0.1.0 - 2026-10-13

Initial release:

- Selected-directory discovery for Node.js, Python and hybrid projects, with every supported runtime declaration evaluated.
- Required environment names, literal file parsing, explicit precedence and diagnostics that omit values.
- Configured loopback port availability and optional TCP reachability, with static and strict modes.
- Human reports and schema 1 JSON with stable check IDs, completeness and exit codes.
- Bounded file and PATH inspection, runtime output and socket probes, including cancellation and child cleanup.
- Source installation, native archive packaging, retained dependency licenses and a synthetic failure-to-correction demo.

Native validation covers macOS arm64, Debian Linux arm64 and Ubuntu Linux x86_64. Hosted macOS and Linux CI passed. Intel macOS and Windows remain unverified. [GitHub release](https://github.com/appdever01/repo-doctor/releases/tag/v0.1.0) includes the native archives, source archive and SHA-256 checksums. No crates.io package is provided.
