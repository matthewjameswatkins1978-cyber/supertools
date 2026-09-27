# Supertools Architecture (v0.1)

One crate, obvious domain boundaries. No workspace, no framework.

## Layout

```text
src/
  main.rs          CLI dispatch (clap) + rendering + exit codes
  cli.rs           command surface only; zero semantics
  output.rs        canonical envelope: Status, Builder, Evidence, Failure
  process.rs       ALL subprocess execution (arg vectors, timeouts, caps)
  catalogue.rs     operation catalogue (static table; drives describe/teach)
  capabilities.rs  readiness model derived from the registry
  doctor.rs        health classification
  describe.rs      catalogue views
  teach.rs         domain guidance + managed AGENTS.md block
  tools_cmd.rs     tools/find commands + install + fix-path flows
  registry/        ONE tool registry (see below)
    mod.rs         ToolId/ToolStatus/ToolRecord, classification, probing
    env.rs         fact collection: PATHs, known dirs, bounded scans
    install.rs     install catalogue + manager dispatch + post-verify
    pathfix.rs     pure USER-PATH planner + Windows applier
  search/mod.rs    rg/fd/ast-grep harnesses + internal fallbacks
  repo/mod.rs      porcelain-v2 model + gh-backed GitHub views
  verify/mod.rs    authority discovery + bounded task execution
```

## One semantic authority, multiple views

- **Tool Registry** (`registry/`) is the only place that knows tool state.
  `tools`, `find`, `doctor`, `capabilities`, install and PATH repair all
  consume it. Domain operations (`search`/`repo`/`verify`) use
  `quick_require()` — same resolution semantics, minimal probing.
- **Operation catalogue** (`catalogue.rs`) is the only place that knows
  what operations exist. `describe`/`teach`/safety tests derive from it.
- **Process module** (`process.rs`) is the only place that spawns
  children. Everything else passes `Request { program, args, cwd,
  timeout, caps }`.

## Discovery stages (registry, Windows-first)

1. current process PATH (order preserved; first match is primary)
2. persisted user PATH, then machine PATH (registry, read-only)
3. exact known developer locations (cargo bin, winget links, scoop shims,
   chocolatey bin, npm global, `~/.local/bin`, …)
4. narrowly bounded scans (depth ≤ 3, 40k-entry budget, name-matched —
   never a full-disk crawl)
5. package-manager metadata (`cargo install --list`, `winget list`;
   notes only, never fabricated paths)

Classification is deterministic: first PATH hit → **available**;
single distinct file off-PATH → **hidden**; several distinct files
off-PATH → **ambiguous**; none → **missing**; found but fails its
`--version` identity check → **broken**. `--version` is only ever run for
candidates from trusted locations; raw-scan hits are reported, never
executed. `sg` is accepted as ast-grep **only** when its version output
identifies as ast-grep (Linux shadow `sg` is excluded with a note).

## Output contract

`output::Builder` builds every result. `--json` prints exactly one
pretty envelope on stdout. Human mode prints compact lines on stdout;
warnings/next-actions go to stderr. Exit code derives from `Status`
(`Ok→0, Failed/Timeout→1, Invalid/Ambiguous/Refused→2,
CapabilityUnavailable→3, NoResults→4`). `ok=true` for both `ok` and
`no_results`.

## Exit-code discipline

- 4 (no matches / nothing applicable) is a *successful completion*.
- 2 covers invalid input, safety refusals, and ambiguity — all cases
  where Supertools declines to guess.
- 3 means the environment lacks a required backend (or doctor finds the
  core too degraded). Missing *optional* tools never produce 3 by
  themselves — they produce degraded capabilities with install guidance.

## Repo parsing notes

`git status --porcelain=v2 --branch -z`, parsed NUL-safely (rename
origins come from the following record; paths may contain spaces/unicode).
Upstream/ahead/behind come from the `# branch.*` headers. In-progress
merge/rebase/cherry-pick/revert/bisect is detected via `.git` sentinel
files. History uses `git log -z` with `\x1f` field separators.

## Verify precedence (documented, tested)

`.supertools.toml [verify]` → single runner (mise/just/package) →
cargo semantics → explicit nothing-found. Two or more runners without
config → `Ambiguous` (exit 2) everywhere except `discover`, which
*explains* the ambiguity (also exit 2). Config-declared task names must
resolve to discovered declared tasks (or fail closed). Execution evidence
is a bounded command record with failure-tail preservation.

## Safety invariants (enforced in code, tested)

- No shell anywhere in product code paths (`process::run` only).
- `search`/`repo` modules contain no mutating git operations; catalogue
  tests assert `read_only` for every search/repo operation id.
- `verify task` resolves against the declared task set; anything else is
  `Refused`.
- Install/fix-path: TTY-or-`--yes` consent, additive-only, re-read
  verification, USER scope only. Non-interactive invocations without
  `--yes` return `Refused` (exit 2), never silent success.
- Hostile-input tests pin: `;`, `&&`, `|`, `$()`, backticks, `>`,
  `%COMSPEC%` are literal data in search; unknown `verify task` names are
  refused.

## Dependencies (conservative)

`clap`, `serde`, `serde_json`, `thiserror`, `which`, `toml`
(+ `winreg` on Windows, `tempfile` for tests). No async runtime, no HTTP
client (Supertools never downloads binaries itself — it dispatches to
existing managers), no regex crate (rg does matching), no terminal crates
(std `IsTerminal` suffices).
