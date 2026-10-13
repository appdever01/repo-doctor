# Repo Doctor

Repo Doctor explains unmet local setup requirements in Node.js and Python projects. Run it after cloning a project to check declared runtime versions, required environment variables and configured loopback ports.

```sh
repo-doctor check
```

Each finding includes its source and a next step when needed. The tool reads the selected directory without running project scripts, installing packages or changing files. It works offline. Passing means the declared checks succeeded; dependencies, builds, credentials and application health need their own checks.

## Install from source

Install stable Rust and Git, then run:

```sh
git clone https://github.com/appdever01/repo-doctor.git
cd repo-doctor
cargo install --path . --locked -j 2
repo-doctor --version
```

Cargo installs into `~/.cargo/bin`; add that directory to PATH if needed. Source installation, tests and release builds are verified on macOS arm64 with Rust 1.97.1 and Debian Linux arm64 in Docker with Rust 1.99.0. The hosted CI workflow targets macOS and Linux; its jobs have not run yet. Windows support is deferred. There is no crates.io package or automatic installer.

## Native archives

Version 0.1.0 archives are prepared for `aarch64-apple-darwin` and `aarch64-unknown-linux-gnu`. They include the binary, build information, examples, report schema and licenses. The Linux build is tested on Debian arm64 and requires glibc; Alpine Linux is unsupported. No x86_64 archive has been verified.

After downloading the matching archive and its `.sha256` file, verify and extract it:

```sh
shasum -a 256 -c repo-doctor-0.1.0-aarch64-apple-darwin.tar.gz.sha256
tar -xzf repo-doctor-0.1.0-aarch64-apple-darwin.tar.gz
./repo-doctor-0.1.0-aarch64-apple-darwin/repo-doctor --version
```

Use the GNU/Linux filenames on Linux; `sha256sum -c` also verifies the checksum there. Move the binary to a directory on PATH if you want to run it from anywhere. These are unsigned command-line binaries; source installation is available if local policy requires signed downloads. Release assets are being prepared and have not been published.

## Use

```sh
repo-doctor check /path/to/project
repo-doctor check --format json
repo-doctor check --strict
repo-doctor check --static
repo-doctor check --services
```

The path defaults to the current directory. Repo Doctor does not search parent directories or recursively scan workspaces. `--static` reads project files and skips host and socket observations. `--services` enables configured TCP connections; static mode takes priority. `--strict` rejects warnings and incomplete coverage as well as failures.

## Supported checks

| Check | Sources and behavior |
| --- | --- |
| Node.js | `package.json.engines.node`, `.nvmrc`, `.node-version`, and explicit configuration. Every declaration is evaluated. |
| Python | `pyproject.toml` with `project.requires-python`, `.python-version`, and explicit configuration. Host Python 3 only. |
| Environment | Names from `.env.example`, plus configured required names. Template values never satisfy a requirement. |
| Package manager | Root npm, pnpm, Yarn and Bun lockfile names and `packageManager`. Launcher presence only; launchers are never executed. Conflicting hints are reported. Bun runtime checks are unsupported. |
| Ports | Temporarily bind explicitly configured loopback ports and release them. Availability can change after the check. |
| Services | With `--services`, attempt a bounded loopback TCP connection. No payload or authentication is sent. |

Node ranges use npm semantics, including exact versions, major ranges, comparators, caret ranges and `||`. Numeric version files accept `22` (stable 22.x), `22.1` (stable 22.1.x) and `v22.1.0` (exact). Named aliases such as `lts/*` remain unverified.

Python requirements use PEP 440 bounds, exclusions and compatible releases. `.python-version` accepts a single numeric version: `3.12` allows patches within 3.12; `3.12.2` pins that version. Multiple interpreters and implementation aliases are unsupported.

Runtime probes use the executable PATH selects. Relative PATH entries, project-local executables and unresolved version-manager shims remain unverified. A later PATH entry cannot hide that result. Python probes try `python3`, then `python` when a trusted candidate is absent or unusable. Node probes accept release version output; custom version suffixes remain unverified. Host executables are a trust boundary; use static mode for file inspection alone.

## Configuration

Create `.repo-doctor.toml` in the selected directory:

```toml
schema = 1

[runtimes]
node = ">=22 <23"

[env]
template = ".env.example"
files = [".env", ".env.local"]
required = ["DATABASE_URL", "SESSION_SECRET"]
optional = ["LOG_LEVEL"]

[[ports]]
name = "api"
address = "127.0.0.1"
port = 3000

[[services]]
name = "postgres"
address = "127.0.0.1"
port = 5432
```

Versions and ports here are examples. A Python requirement uses `python = ">=3.11,<3.14"` under `[runtimes]`. Configuration adds requirements without replacing manifests. Unknown fields, unsupported schemas, duplicate check names, invalid ports and remote addresses are rejected. Check names use lowercase letters, digits and hyphens. Addresses are loopback IP literals such as `127.0.0.1` or `::1`.

File paths are relative to the selected project. Absolute paths, parent escapes and symlinks leading outside it are rejected. Configuration contains requirements and variable names, never credentials.

## Environment rules

By default, `.env` loads first, `.env.local` overrides it, and process variables override both. An empty higher-priority value stays empty. This is Repo Doctor's convention; list different files if your framework uses another order.

The parser supports blank lines, comments, key-value assignments, `export`, and single or double quotes. Double quotes accept `\\`, `\"`, `\n`, `\r`, `\t` and `\$` escapes. Inline comments follow whitespace or a closing quote. Duplicate keys use the last assignment and produce a warning. References and command substitutions remain literal. Multiline values and unsupported syntax produce a sanitized error. A malformed value file leaves affected requirements unverified unless the process environment supplies the final value.

An absent default template declares no names. An explicitly selected missing template is an error. Missing value files can be satisfied by process variables. Required names that are missing or empty fail; optional names may be absent. Reports contain names and source filenames without values, lengths, hashes or raw parser lines.

## Reports and exit codes

Human and JSON output come from the same report. JSON writes one object to stdout and follows [schema 1](schemas/report-v1.schema.json). Check IDs and ordering are deterministic. The project label is neutral and evidence paths are relative.

| Exit | Meaning |
| --- | --- |
| 0 | No failed or errored checks; ordinary mode allows warnings and incomplete coverage. |
| 1 | A failed or errored check, or strict mode rejected warnings or incomplete coverage. |
| 2 | Invalid arguments, invalid configuration or an unusable target. |
| 3 | An internal failure prevented a reliable report or output. |

Outcomes are `passed`, `issues_found`, `incomplete`, `invalid_input` and `internal_error`. Statuses are `pass`, `warn`, `fail`, `skipped` and `error`. Use the outcome and `complete` field alongside the exit code. Zero with skipped checks still means incomplete coverage. A runtime without a version requirement gets a compatibility warning and incomplete coverage.

## Limits

Checks run sequentially. Runtime probes have a 3-second deadline, TCP connections 500 milliseconds, and the run a 15-second deadline. Each input file is limited to 1 MiB, combined contents to 16 MiB, captured output to 16 KiB per stream, and port and service checks to 32 combined. There are at most 32 environment value files. Errors and unfinished checks stay visible. Child processes are terminated and reaped, and sockets are released.

File and PATH inspection use bounded worker waits, so a stalled filesystem operation produces an incomplete report at the run deadline. The command does not wait for a stalled worker during shutdown. Probes clear inherited startup variables and use fixed arguments. No remote network request is made by the tool; service checks stay on loopback.

Version 0.1.0 does not inspect virtual environments, installed packages, Docker, recursive monorepos or database credentials. An accepted TCP connection establishes reachability only.

## Demo and development

The [synthetic demo fixture](examples/demo-api) contains a deliberately mismatched Node requirement and a missing variable. Its README explains how to inspect a copy and correct it. Workflow tests also hold a temporary port open, verify the failure, release it and confirm the corrected setup passes.

```sh
cargo test --locked -j 2 -- --test-threads=2
cargo fmt --check
cargo clippy --locked -j 2 --all-targets -- -D warnings
cargo build --locked --release -j 2
sh scripts/package-release.sh
```

The binary uses a small library. Discovery retains declarations, check modules interpret them, and bounded host adapters collect observations. Both renderers share a report and exit policy. Tests cover parsing, subprocess and socket behavior, schema validation, safety boundaries and corrected-project workflows.

Repo Doctor's original work is the diagnostic workflow and report policy. [clap](https://docs.rs/clap/) supplies argument parsing, [node-semver](https://docs.rs/node-semver/) supplies npm ranges, and [pep440_rs](https://docs.rs/pep440_rs/) supplies Python specifiers. The local environment parser preserves literal values and sanitized errors. [mise](https://github.com/jdx/mise) informed the brief; no mise source was copied. See [dependency notices](THIRD_PARTY_NOTICES.md) for retained licenses and credits. Original code is MIT licensed.
