---
name: audit
description: >-
  Project conformance and efficiency audit. Verifies that each part of a
  codebase actually does what its name, docs, types, tests, and contracts
  claim it does, and rates how efficiently it does it. Produces a tiered
  conformance ledger with per-part coverage and a verdict, then offers to
  reconcile confirmed mismatches. Use when inheriting a codebase, before
  a release, or when behavior and documentation may have drifted apart.
user-invocable: true
disable-model-invocation: true
allowed-tools: Agent Bash(git *) Bash(pnpm *) Bash(npm *) Bash(pytest*) Bash(cargo *) Bash(go *) Read Grep Glob Edit AskUserQuestion
argument-hint: "[path] [--diff|--staged] [--threshold warning|critical] [--focus conformance|efficiency] [--fix]"
---

# Audit — Conformance and Efficiency

You orchestrate a **project audit** that answers two questions about
every part of the codebase:

1. **Conformance** — does this unit actually do what its name, docs,
   type signature, schema, and tests claim it does?
2. **Efficiency** — given that it does it, does it do it with reasonable
   algorithmic and I/O cost?

Scope boundaries with sibling tools:
- `/code-review` is diff-scoped; this skill audits the project as it
  stands, regardless of recent changes.
- `/complexity-audit` owns exhaustive per-function Big O analysis; this
  skill flags efficiency problems it trips over while verifying behavior
  and defers deep algorithmic sweeps there.
- `/optimize` and `/n-plus-one` own the deep performance dives.
- `/tech-debt` and `/simplify` own debt and over-engineering.

This skill's unique job is the **claim-vs-implementation gap**: functions
that validate inputs and then don't do the work, names and docs that
describe behavior the code no longer has, advertised options that are
ignored, contracts the implementation silently violates.

## Inputs

- `$ARGUMENTS` — optional. Accepts a path, diff scope, threshold, focus, and fix flag.

Flags:
- Path: relative directory to scope the audit (default: entire project)
- `--diff` — audit only files in unstaged diff (`git diff --name-only`)
- `--staged` — audit only files in staged diff (`git diff --cached --name-only`)
- `--threshold warning` — hide Info findings
- `--threshold critical` — hide Info and Warning findings
- `--focus conformance` — skip efficiency checks
- `--focus efficiency` — skip conformance checks
- `--fix` — jump straight to the reconciliation offer after the report

Parse `$ARGUMENTS` into:
- `SCOPE_PATH` — directory path, or `.` if not provided
- `DIFF_MODE` — `none` (default), `unstaged`, or `staged`
- `THRESHOLD` — `info` (default), `warning`, or `critical`
- `FOCUS` — `both` (default), `conformance`, or `efficiency`
- `FIX_JUMP` — true if `--fix` was passed

Examples:
- `/audit` → whole project, both focuses, all tiers
- `/audit src/services/` → scoped to a directory
- `/audit --diff` → audit only unstaged changes
- `/audit --staged` → audit only staged changes
- `/audit --diff --focus conformance` → claim-vs-implementation only on unstaged files
- `/audit --threshold warning --fix` → hide Info, jump to reconciliation

## Phase 1: Inventory

### 1.1 Detect languages and file list

Parse `SCOPE_PATH` and `DIFF_MODE` from `$ARGUMENTS` first.

**If `DIFF_MODE` is `unstaged`:**

```bash
git diff --name-only -- "<SCOPE_PATH>" | grep -vE '(\.md$|\.txt$|\.csv$|\.svg$|\.png$|\.jpg$|\.gif$|\.ico$|\.woff|\.eot$|\.ttf$|\.map$|\.min\.|\.lock$|node_modules/|vendor/|dist/|build/|\.git/|__pycache__|\.pytest_cache|\.next/|\.nuxt/|target/|\.idea/|\.vscode/)'
```

**If `DIFF_MODE` is `staged`:**

```bash
git diff --cached --name-only -- "<SCOPE_PATH>" | grep -vE '(\.md$|\.txt$|\.csv$|\.svg$|\.png$|\.jpg$|\.gif$|\.ico$|\.woff|\.eot$|\.ttf$|\.map$|\.min\.|\.lock$|node_modules/|vendor/|dist/|build/|\.git/|__pycache__|\.pytest_cache|\.next/|\.nuxt/|target/|\.idea/|\.vscode/)'
```

**If `DIFF_MODE` is `none` (default):**

```bash
git ls-files -- "<SCOPE_PATH>" | sed 's/.*\.//' | sort | uniq -c | sort -rn | head -20
git ls-files -- "<SCOPE_PATH>" | grep -vE '(\.md$|\.txt$|\.csv$|\.svg$|\.png$|\.jpg$|\.gif$|\.ico$|\.woff|\.eot$|\.ttf$|\.map$|\.min\.|\.lock$|node_modules/|vendor/|dist/|build/|\.git/|__pycache__|\.pytest_cache|\.next/|\.nuxt/|target/|\.idea/|\.vscode/)' | head -1000
```

If the command returns no files, stop and tell the user:
- For diff mode: no changed files match the scope
- For project mode: the path contains no tracked code files

Store as `CODE_FILES`. Keep test files in the list — tests are evidence
of claimed behavior, and untested claims are findings.

### 1.2 Gather intent sources

The audit compares implementation against **stated intent**. Collect the
places intent is stated:

- `README.md`, `docs/` — read the sections describing what modules do
- API contracts: `schema.gql`, `*.graphql`, OpenAPI specs, `*.proto`
- Project conventions: `CLAUDE.md`, `AGENTS.md`, `CONTRIBUTING.md`
- Package manifests: `description` fields, exported entry points

Store relevant excerpts as `INTENT_SOURCES`. Project conventions
OVERRIDE generic expectations — a pattern the project mandates is
conformant by definition.

### 1.3 Partition into parts

Group `CODE_FILES` by module boundary (top-level directories under
`SCOPE_PATH`, or per-package in a monorepo). Each part goes to one
agent. Agent count by code-file total:

| Code files | Agents |
|-----------|--------|
| < 50      | 2      |
| 50–150    | 3      |
| 150–300   | 4      |
| 300–500   | 6      |
| 500+      | 8      |

Never exceed 8. Keep a directory's files in the same partition.

## Phase 2: Parallel verification

Launch all agents **in parallel** in a single message, each with
`subagent_type: "Explore"` (read-only). Prompt template:

```
You are a conformance and efficiency auditor. For each unit in your
assigned files, verify that it does what it claims to do, and assess
how efficiently it does it. Report findings only; modify nothing.

## Project context

- Scope: {SCOPE_PATH}
- Focus: {FOCUS}
- Threshold: {THRESHOLD}
- Stated intent (docs/contracts/conventions — conformance is judged
  against THESE, and project conventions override generic expectations):
  {INTENT_SOURCES}

## Your files

{AGENT_N_FILES — one per line}

## Method

For each file, enumerate its units: exported functions, methods,
classes, endpoints, resolvers, handlers, jobs, CLI commands. For each
unit, do a three-step check:

### Step 1 — Establish the claim

What does this unit promise? Sources, in priority order:
1. An external contract it implements (GraphQL schema field, OpenAPI
   operation, proto RPC, interface it satisfies)
2. Its docstring / doc comment
3. Its type signature (params, return type, error type)
4. Its name (`validateX` claims validation, `getUserById` claims a
   lookup by id, `retryWithBackoff` claims backoff)
5. Documentation that mentions it (from INTENT_SOURCES)

If no claim can be established (anonymous glue code, trivial
pass-through), skip the unit — it can't be non-conformant.

### Step 2 — Verify the implementation (skip if FOCUS=efficiency)

Read the body. Trace one level into same-file helpers it calls. Compare
against the claim. Mismatch categories:

- **Hollow**: validates inputs or acquires resources, then returns
  success without doing the claimed work.
- **Divergent**: does something other than the claim — name/doc says X,
  code does Y (e.g., `sortByDate` sorts by id; doc says "throws on
  invalid input" but it returns null).
- **Partial**: advertised parameters, options, flags, or branches are
  accepted but ignored; documented edge cases unhandled.
- **Contract violation**: return shape, nullability, error behavior, or
  side effects differ from the schema/interface/type it implements.
- **Silent failure**: the claim implies an error surfaces, but the code
  swallows it (empty catch, ignored error return, `.catch(() => {})`).
- **Stale claim**: implementation is correct and sensible, but the
  name/doc/contract describes an older behavior. The CODE is fine; the
  CLAIM is the bug. Mark these `reconcile: claim`.

Then check the test evidence: does any test actually assert the claimed
behavior? Grep for the unit's name in test files and read the matching
tests. A unit whose claim is load-bearing (contract, exported API) with
no test asserting it → Info finding ("unverified claim"), even when the
code looks correct.

### Step 3 — Assess efficiency (skip if FOCUS=conformance)

While the body is in front of you, check its cost:
- Algorithmic: nested loops over the same data, linear scans inside
  loops (`.includes`/`.indexOf`/`in` on arrays), sort-inside-loop,
  repeated recomputation of an invariant.
- I/O: queries or network calls inside loops, per-item awaits that
  could batch, the same read repeated within one call path, missing
  obvious caching for hot repeated work.
- Only flag what you can see from the code. Do not run profilers. For
  systemic issues (project-wide N+1s, bundle weight), note once that
  /optimize or /n-plus-one is the right tool and move on.

## Severity tiers

| Tier | Criteria |
|---|---|
| Critical | Hollow or Divergent units on exported/contract surfaces. Contract violations. Silent failures on paths where callers depend on the error. Efficiency: unbounded I/O-in-loop on a request path. |
| Warning | Partial conformance (ignored options, unhandled documented cases). Hollow/Divergent on internal units. Efficiency: O(n²)+ over unbounded input, redundant repeated I/O. |
| Info | Stale claims (code right, doc/name wrong). Unverified claims (no test asserts a load-bearing claim). Efficiency: avoidable recomputation with modest cost. |

Apply THRESHOLD: `warning` drops Info; `critical` drops Info and Warning.

## Output format

### Part summary

```
### Part: {directory} — Agent {N}

- Units checked: <count>
- Conformant: <count>
- Findings: <C> Critical · <W> Warning · <I> Info
```

### Finding block (one per finding)

```
#### [CRITICAL|WARNING|INFO] <unit name> — `path/to/file.ext:LINE`

- **Type**: hollow | divergent | partial | contract | silent-failure |
  stale-claim | unverified-claim | efficiency
- **Claim**: <what it promises, and where the claim comes from>
- **Actual**: <what the code actually does, with a 3-10 line evidence
  snippet>
- **Test evidence**: <asserting test file:line, or "none">
- **Reconcile**: code | claim | test — which side should change
- **Fix**: <concrete steps>
```

## Important

- A finding needs evidence from the code you read. No claim source, no
  finding. Uncertain about intent → say so and downgrade one tier.
- Do NOT flag style, naming taste, or debt — sibling tools own those.
- Do NOT modify any files. Read-only.
```

## Phase 3: Report assembly

After all agents return, print the unified report inline (never to a
file):

```markdown
# Project Audit — {scope}

**Units checked:** <N> across <M> files ({languages})
**Conformant:** <N> ({percent}%)
**Findings:** <C> Critical · <W> Warning · <I> Info

> Static analysis by Claude against stated intent — not formal
> verification. Claims were taken from contracts, docs, signatures,
> and names; ambiguous intent is noted, not assumed.

## Conformance ledger

| Tier | Unit | Location | Type | Reconcile | Summary |
|---|---|---|---|---|---|

<full finding blocks, sorted Critical → Warning → Info>

## Part coverage

| Part | Units | Conformant | Worst finding |
|---|---|---|---|

## Verdict

<one of: BLOCK | APPROVE WITH FIXES | APPROVE>
```

**Verdict rules:** any Critical → **BLOCK**; Warnings only →
**APPROVE WITH FIXES**; Info only or none → **APPROVE**.

If zero findings: print "Every audited unit does what it claims. Ship
it." and stop.

## Phase 4: Reconciliation

Skip this phase if there are zero findings. If `FIX_JUMP` is true, skip
the offer prompt.

Otherwise ask with AskUserQuestion:

> "Reconcile the findings? Each one names which side should change
> (code, claim, or test)."

Options:
1. **All** — apply every finding's reconciliation
2. **Critical + Warning only** — skip Info-tier
3. **Pick specific findings** — numbered selector, comma-separated
4. **Skip** — keep the report as-is

### 4.1 Dispatch fix agents

Group selected findings by file. One Agent per file in parallel,
`subagent_type: "general-purpose"`:

```
You are reconciling audit findings in a single file. Each finding names
which side changes:

- `reconcile: code` — make the implementation match the claim. This IS
  a behavior change by design; change exactly the behavior named in the
  finding and nothing else. Never change a public signature — if the
  fix requires one, skip and report "needs manual review".
- `reconcile: claim` — fix the doc/comment/name to describe the actual
  behavior. Renames only when every reference is in this repo; update
  all references, or skip.
- `reconcile: test` — add a test asserting the claimed behavior,
  following the project's existing test patterns and file layout.

## File
{file_path}

## Findings
{finding blocks for this file}

Use Read then Edit. Return per finding: applied (what changed) or
skipped (why).
```

### 4.2 Verify and stage

Detect and run the project test command (first match): `package.json`
scripts.test → `pnpm test` (or `npm test` without a pnpm-lock);
pytest config → `pytest`; `Cargo.toml` → `cargo test`; `go.mod` →
`go test ./...`. Then type-check where available (`pnpx tsc --noEmit`,
`cargo check`, `mypy .`).

On any failure: revert the touched files with `git checkout -- <paths>`,
print the failing output and the findings that could not be applied,
and stop.

On success, stage only the edited files by path and print:

```markdown
## Reconciled

| File | Applied | Skipped | Side changed |
|---|---|---|---|

Changes are staged. Review with `git diff --cached` and commit when
ready. **No commit was made.**
```

List skipped findings with reasons under "Skipped — needs manual
review".

## Safety rails

- **Read-only through Phase 3.** Verification agents use only Read,
  Grep, Glob, and read-only git commands. No profilers, no installs,
  no network.
- **`reconcile: code` changes behavior on purpose** — that's the point
  of the skill — but only the exact behavior a finding names, never a
  public signature, and never without the finding's claim as warrant.
- **Reconciliation touches only files cited in findings.**
- **Revert on any verification failure** — tests or type-check.
- **No commits, no pushes, no PRs.** Leave changes staged.
- **Project conventions override generic expectations.** Rules from
  `CLAUDE.md`, `AGENTS.md`, and `CONTRIBUTING.md` define conformance.
