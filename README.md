# Supertools

**Agent-native harnesses for existing developer tools.**

Supertools is a small, fast, cross-platform Rust CLI that makes common
developer tools easier, safer and more predictable for AI coding agents to
operate. It does **not** replace Git, ripgrep, Cargo, GitHub CLI, mise,
just, ast-grep or Threadmoth — it provides a thin agent-facing operational
layer over them:

> Existing tools expose capabilities.
> Supertools exposes intent, bounded operations, structured evidence,
> safe defaults, and enough operational teaching for an agent to use
> those capabilities correctly.

An unfamiliar agent needs only to know Supertools exists; the executable
itself teaches the rest (`capabilities`, `describe`, `teach`).

## What it deliberately is NOT

- Not a search engine, build system, task runner, or Git replacement.
- No MCP server, GUI, daemon, database, index, embeddings, LLM calls,
  telemetry, or cloud anything (v0.1).
- No source mutation, no Git mutation: `search` and `repo` are read-only;
  `verify` only invokes the project's own existing verification commands.

## Install

Requires Rust (stable) to build. User-level install, nothing machine-wide:

```powershell
cargo install --path .
```

This drops `supertools.exe` into `~/.cargo/bin` (already on PATH for most
Rust setups). Verify from a fresh shell:

```powershell
supertools --version
supertools doctor
```

## 60-second usage

```powershell
supertools doctor                 # is anything actually wrong?
supertools capabilities           # what can I actually do right now?
supertools describe search.text   # exact schema for one operation
supertools teach search           # compact workflow guidance

supertools search text "Permission"
supertools search files "invoice"
supertools search context "TODO" --context 2
supertools search symbol "calculate_total"
supertools search structural "fn $NAME($$$) -> u32 { $$$ }"

supertools repo state             # branch, HEAD, dirty counts, upstream, op-state
supertools repo changed
supertools repo history --limit 5
supertools repo remote
supertools repo pr                # needs gh + GitHub remote + auth
supertools repo ci

supertools verify discover        # how does THIS repo check itself?
supertools verify quick           # cheapest authoritative check
supertools verify full            # strongest local gate
supertools verify task check      # one declared task, exact name only

supertools tools                  # inventory: what tools does this machine have?
supertools find fd                # deep investigation: where/why invisible/what to do
supertools tools install-missing  # consent-gated install + re-discovery verify
supertools tools fix-path --dry-run
```

## JSON mode

Every command accepts `--json` and emits one canonical envelope on stdout
(no banners, no ANSI, diagnostics go to stderr in human mode):

```json
{
  "schema_version": 1,
  "tool": "supertools",
  "version": "0.1.0",
  "operation": "repo.state",
  "ok": true,
  "status": "ok",
  "summary": "branch main — clean",
  "data": { "...typed per-operation payload..." },
  "evidence": [{ "kind": "command", "program": "git", "args": ["status", "--porcelain=v2", "--branch", "-z"], "exit_code": 0, "duration_ms": 41 }],
  "warnings": [],
  "next_actions": [{ "command": "supertools repo changed", "reason": "..." }],
  "truncated": false
}
```

`status` is one of `ok | no_results | failed | timeout | invalid_request |
capability_unavailable | ambiguous | refused`. Exit codes: `0` success ·
`1` execution/verification failure (incl. timeout) · `2`
invalid/refused/ambiguous · `3` capability unavailable · `4` completed but
no matches / nothing applicable. (`no_results` has `ok: true`.)

## Human presentation

When — and only when — stdout is an attached terminal in human mode, output
is dressed by [Sartorial](https://github.com/matthewjameswatkins1978-cyber/Sartorial)
(pinned revision, House preset). Sartorial never becomes semantic authority:
piped/redirected output stays plain and deterministic, `--json` never passes
through it, and exit codes, warnings and next actions are unchanged.

## Doctor

`supertools doctor` answers "is anything actually wrong?" with one of
`healthy | healthy_with_optional_gaps | degraded | broken` plus checks and
optional improvements. A missing optional backend never means "broken".

## Self-teaching

The recommended persistent agent instruction stays tiny (see
`supertools teach install` for the exact block + managed install into
Codex/OpenCode global `AGENTS.md`):

```text
Supertools is installed locally.

Before manually constructing repository search, Git-state,
or verification command sequences, prefer:

    supertools search
    supertools repo
    supertools verify

If unfamiliar with an operation, run:

    supertools teach <domain>
    supertools describe <operation>

Fall back to the native underlying tool when Supertools does
not support the required operation.
```

## Optional dependencies

Supertools capability-detects and degrades gracefully. Recommended
"agent workstation" toolset: `git cargo rustc rg fd gh mise just
ast-grep/sg threadmoth`. `supertools tools` shows discovered state
(`available | hidden | missing | ambiguous | broken`); `supertools find`
investigates; `supertools tools install-missing` installs via trusted
existing package managers (with consent, verified by re-discovery).

Graceful fallbacks: text search without `rg` uses a bounded internal
fixed-substring scan (regex then requires `rg`); file search without `fd`
uses bounded `git ls-files`; structural search without `ast-grep` reports
unavailable and points at textual search without pretending equivalence.

## Safety model

- Argument-vector subprocess execution only; queries are data, never shell.
- `search`/`repo` strictly observational. No stage/commit/merge/push/config
  mutation exists anywhere in v0.1.
- `verify task <name>` must resolve to a *discovered declared task*;
  arbitrary command strings are refused (exit 2).
- Installation and PATH repair require explicit consent (prompt or `--yes`),
  are additive-only, verified by re-reading, and never touch machine PATH.
- All output is bounded; truncation is always explicit.

## Verification discovery

Precedence (see `supertools verify discover`): explicit `.supertools.toml`
`[verify]` config wins; otherwise a single task-runner authority at the
repo root (`mise.toml` / `justfile` / `package.json` scripts); multiple
runners with no config → **ambiguous, fail closed (exit 2)**; otherwise
`Cargo.toml` → cargo semantics (`quick` = `cargo check --all-targets`,
`full` = `cargo test`); nothing → reported explicitly (exit 4).

```toml
# .supertools.toml — only needed to resolve what cannot be safely inferred
[verify]
authority = "just"   # mise | just | package | cargo
quick = "check"      # must name a declared task
full = "test"        # must name a declared task
timeout_seconds = 900

[limits]
output_bytes = 8192  # diagnostic tail cap for verify runs
```

## Limitations (v0.1, honest)

- Discovery/PATH repair are Windows-first; other platforms get shallow
  probing and clean "unsupported" messages.
- Internal text fallback is fixed-substring only, git-listed files only.
- `search.symbol` is a labeled heuristic, not semantic resolution.
- `repo pr/ci` need `gh` + GitHub remote + authentication.
- Package-script verification needs an npm/pnpm/yarn/bun runner for
  execution (discovery works regardless).
- `repo show <sha|file>` and bounded blame are deliberately NOT in v0.1 —
  parked as candidates for v0.2.
- On Windows, running `supertools verify full` from a candidate binary that
  lives inside `target/` of the same workspace can fail while cargo replaces
  the locked exe; run the candidate from a copy outside `target/`.

## Future: MCP

No MCP server in v0.1 by design. The operation catalogue carries
`read_only / destructive / idempotent` metadata and every operation has a
stable typed schema precisely so an MCP adapter can later expose the same
operations without redesigning the core.

## Development

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
git diff --check
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for module layout and
design decisions. CI runs Windows + Ubuntu (macOS skipped: runner cost;
it should work where the backends exist).
