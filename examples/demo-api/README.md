# Demo API fixture

This is synthetic setup data, not a runnable API. It deliberately requests Node.js 0.0.1 and omits a required environment variable.

Copy this folder into a temporary directory so demonstration changes stay separate:

```sh
demo_dir=$(mktemp -d)
cp -R examples/demo-api/. "$demo_dir/"
repo-doctor check "$demo_dir"
```

The first run should report a version mismatch and the missing `REPO_DOCTOR_DEMO_SESSION_SECRET`. Select a trusted host Node executable first if PATH uses an unresolved shim. Edit the copy's `package.json` to request your installed Node version, and create `.env` in the copy:

```dotenv
REPO_DOCTOR_DEMO_SESSION_SECRET=synthetic-demo-value
```

Run `repo-doctor check "$demo_dir"` again. The declared checks should pass. `--static` skips the runtime probe; `--strict` rejects its incomplete coverage. Automated tests also demonstrate an occupied port using a temporary listener without a fixed test port.
