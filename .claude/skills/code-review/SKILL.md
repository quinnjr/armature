---
name: code-review
description: >-
  Expert, language- and framework-agnostic code review. Orchestrates 7
  parallel specialist agents across correctness, security, maintainability,
  API & contracts, testing & verifiability, error handling & resilience,
  and tech debt.
  Auto-detects the diff range (uncommitted, staged, commit range, or unpushed
  branch) and reviews only what changed, with the surrounding code loaded for
  context. Reports findings in severity tiers with concrete, actionable
  fixes and optionally applies them. Use before committing, before pushing,
  before opening a PR, or when asked for a second opinion on recent work.
user-invocable: true
disable-model-invocation: true
allowed-tools: Agent Bash(git *) Bash(cat *) Bash(ls *) Bash(rg *) Bash(wc *) Read Grep Glob Edit AskUserQuestion
argument-hint: "[scope: unstaged | staged | branch | HEAD~N..HEAD | <path>] [--domains correctness,security,maintainability,api,testing,errors,debt | all] [--threshold critical|high|medium|low] [--fix]"
---

# Code Review — Expert Multi-Agent Review

You are orchestrating an **expert, language- and framework-agnostic code
review** using 7 parallel specialist agents. Each agent owns one review
dimension and reports findings in severity tiers with concrete, actionable
fixes. After the report you offer to implement the fixes.

This skill reviews **change**, not the whole repository. The target is
whatever set of modifications the user wants assessed — uncommitted work,
a staged hunk, a range of commits, an unpushed branch, or a specific path.
It is deliberately distinct from `/complexity-audit`, `/optimize`, and
`/security-hardening` (which are full-project audits) and from the
`critical-reviewer` agent (a single hostile reviewer). This skill is the
multi-dimensional review a senior engineer runs before approving a PR.

## Inputs

`$ARGUMENTS` is free-form. Parse it for:

- **Scope** (what to review). Accepts:
  - `unstaged` — `git diff` (default when nothing is passed and the working
    tree is dirty)
  - `staged` — `git diff --cached`
  - `branch` — all commits on the current branch not yet on the upstream
    (or `develop`/`main` if no upstream): `git diff <base>...HEAD`
  - A git range like `HEAD~3..HEAD` or `abc123..HEAD` or `abc123..def456`
  - A path (file or directory) — diff is `git diff -- <path>` scoped to
    unstaged changes under that path
- **`--domains`**: comma-separated subset of `correctness`, `security`,
  `maintainability`, `api`, `testing`, `errors`, `debt`, or `all`
  (default).
- **`--threshold <tier>`**: minimum tier to display. Values: `critical`,
  `high`, `medium`, `low` (default: `low` — show all).
- **`--fix`**: proceed straight to the fix offer after the report (skips
  the intermediate confirmation step).

Parse into:
- `SCOPE_MODE` — `unstaged` | `staged` | `branch` | `range` | `path`
- `SCOPE_REF` — git range string (e.g., `HEAD`, `abc123..HEAD`, `--cached`)
- `SCOPE_PATH` — optional path filter
- `DOMAINS` — list of domain keys
- `THRESHOLD` — `critical` | `high` | `medium` | `low`
- `FAST_FIX` — boolean

Examples:
- `/code-review` → auto-detect (unstaged if dirty, else branch)
- `/code-review staged` → staged changes only
- `/code-review branch` → everything on this branch vs upstream
- `/code-review HEAD~5..HEAD` → last 5 commits
- `/code-review src/auth/` → unstaged changes under `src/auth/`
- `/code-review --domains correctness,security` → two domains only
- `/code-review --threshold high` → hide medium + low
- `/code-review branch --fix` → review and jump to fix offer

## Phase 1: Scope Resolution

Determine the diff range before anything else. This drives every agent.

IMPORTANT: every `!` block in this skill spawns a fresh shell. Shell
variables set in one block do NOT survive to the next. To work around
that, the scope-resolution block below prints all the values you need;
you (the model) should READ those values from its output and then
substitute them as string literals into the subsequent commands before
executing them via the Bash tool. Do NOT try to reference shell variables
like `$SCOPE_REF` across blocks — they will be empty.

### 1.1 Resolve scope in a single block

```!
set -e
MODE="" SCOPE_REF="" SCOPE_PATH="" BASE="" BASE_SHA=""

# Try to treat the command-line argument as a git range or path. When
# this skill is invoked with an argument ($ARG below), substitute it
# here. Otherwise auto-detect.
ARG=""

if [ -n "$ARG" ]; then
  case "$ARG" in
    staged)  MODE=staged; SCOPE_REF="" ;;
    branch)  MODE=branch ;;
    *..*)    MODE=range; SCOPE_REF="$ARG" ;;
    *)
      if [ -e "$ARG" ]; then
        MODE=path; SCOPE_PATH="$ARG"
      else
        echo "ERROR: unrecognised scope argument: $ARG" >&2
        exit 1
      fi
      ;;
  esac
else
  if [ -n "$(git status --porcelain 2>/dev/null | head -1)" ]; then
    MODE=unstaged
  else
    MODE=branch
  fi
fi

if [ "$MODE" = "branch" ]; then
  BASE=$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null \
      || git rev-parse --verify origin/develop 2>/dev/null \
      || git rev-parse --verify develop 2>/dev/null \
      || git rev-parse --verify origin/main 2>/dev/null \
      || git rev-parse --verify main 2>/dev/null \
      || echo "")
  if [ -n "$BASE" ]; then
    BASE_SHA=$(git merge-base HEAD "$BASE")
    SCOPE_REF="${BASE_SHA}..HEAD"
  else
    # Detached HEAD / fresh clone: review just the tip commit.
    SCOPE_REF="HEAD~1..HEAD"
  fi
fi

echo "MODE=$MODE"
echo "SCOPE_REF=$SCOPE_REF"
echo "SCOPE_PATH=$SCOPE_PATH"
echo "BASE=$BASE"
echo "BASE_SHA=$BASE_SHA"
```

Read those five values out of the output. From here on, when a command
below shows `$MODE`, `$SCOPE_REF`, or `$SCOPE_PATH`, substitute the
literal value you saw (e.g. rewrite `"$SCOPE_REF"` as
`"abc123..HEAD"`). That gives each command a self-contained shell
invocation that does not depend on any persisted variable.

### 1.2 Gather the diff

Run exactly one of the following, chosen by the `MODE` value from 1.1.
Substitute the literal `SCOPE_REF` / `SCOPE_PATH` strings; do not rely
on shell variables.

```
# MODE=unstaged (no path scope):
git diff

# MODE=unstaged with SCOPE_PATH=src/foo/:
git diff -- src/foo/

# MODE=staged:
git diff --cached

# MODE=branch or range (e.g. SCOPE_REF=abc123..HEAD):
git diff abc123..HEAD

# MODE=path (SCOPE_PATH=src/foo/):
git diff -- src/foo/
```

Store the output as `DIFF`. If the diff is empty, stop and tell the user
there is nothing to review.

Get the list of changed files with the same substitution rule, adding
`--name-only` to the chosen command (e.g. `git diff --name-only abc123..HEAD`).

If the diff is very large (say, >2000 lines or >50 files), warn the
user and ask whether to proceed, narrow the scope, or split.

### 1.3 Commit messages in scope

For `branch` or `range` mode, grab the commit messages so agents can
correlate findings with claimed intent. Substitute the literal
`SCOPE_REF`:

```
# Example when SCOPE_REF=abc123..HEAD:
git log --format='%h %s%n%n%b%n---' abc123..HEAD
```

Store as `COMMIT_MESSAGES`. Skip this step if `MODE` is `unstaged`,
`staged`, or `path`.

## Phase 2: Context Assembly

Each agent needs the change AND the surrounding code. A diff alone hides
call sites, type definitions, and conventions.

### 2.1 Detect languages and manifests

```!
printf '%s\n' $CHANGED_FILES | sed 's/.*\.//' | sort | uniq -c | sort -rn
```

```!
ls -1 package.json pnpm-lock.yaml Cargo.toml go.mod pyproject.toml requirements.txt Gemfile composer.json pom.xml build.gradle 2>/dev/null || echo "no manifests"
```

Store as `LANGUAGES` and `MANIFESTS`.

### 2.2 Framework and convention signals

```!
rg -l --max-count=1 "express|fastify|nestjs|@nestjs|angular|react|vue|svelte|next|nuxt|remix|axum|actix|gin-gonic|flask|django|fastapi|rails|spring" -g '!node_modules' -g '!target' -g '!dist' -g '!vendor' 2>/dev/null | head -10 || echo "no framework detected"
```

```!
ls -1 CLAUDE.md AGENTS.md CONTRIBUTING.md .editorconfig .prettierrc* .eslintrc* rustfmt.toml .rubocop.yml .golangci.yml 2>/dev/null || echo "no convention files"
```

Store as `FRAMEWORKS` and `CONVENTIONS`.

### 2.3 Project instructions

If `CLAUDE.md`, `AGENTS.md`, or `CONTRIBUTING.md` exists at the repo root,
read the top 200 lines of whichever is present and store as
`PROJECT_INSTRUCTIONS`. Agents must respect these — they encode the user's
actual rules and override generic best-practice advice.

## Phase 3: Agent Dispatch

Launch the selected agents **in parallel** in a single message containing
multiple Agent tool calls. Use `subagent_type: "Explore"` for each
(read-only).

| Domain key         | Agent prompt file                                           | Owns                                                    |
| ------------------ | ----------------------------------------------------------- | ------------------------------------------------------- |
| `correctness`      | `${CLAUDE_SKILL_DIR}/agents/correctness.md`                 | Logic bugs, off-by-one, race conditions, edge cases     |
| `security`         | `${CLAUDE_SKILL_DIR}/agents/security.md`                    | Injection, auth/authz, secrets, SSRF, unsafe defaults   |
| `maintainability`  | `${CLAUDE_SKILL_DIR}/agents/maintainability.md`             | Readability, duplication, naming, scope creep, dead code|
| `api`              | `${CLAUDE_SKILL_DIR}/agents/api-contract.md`                | Signature changes, breaking changes, contract drift     |
| `testing`          | `${CLAUDE_SKILL_DIR}/agents/testing.md`                     | Missing/weak tests, untestable code, flaky patterns     |
| `errors`           | `${CLAUDE_SKILL_DIR}/agents/error-handling.md`              | Silent failures, swallowed errors, missing timeouts     |
| `debt`             | `${CLAUDE_SKILL_DIR}/agents/tech-debt.md`                   | TODO/HACK markers, suppressions, deprecated usage, temporary code |

Prefix every agent prompt with this shared context block:

```
## Review Context
- **Scope**: <SCOPE_MODE> (<SCOPE_REF>)
- **Path filter**: <SCOPE_PATH or "none">
- **Changed files**: <CHANGED_FILES>
- **Languages**: <LANGUAGES>
- **Manifests**: <MANIFESTS>
- **Framework signals**: <FRAMEWORKS>
- **Convention files**: <CONVENTIONS>
- **Project instructions (excerpt)**:
<PROJECT_INSTRUCTIONS — or "none">

## Commit messages in scope
<COMMIT_MESSAGES — or "none; uncommitted changes">

## Diff under review
```diff
<DIFF>
```

## Instructions
Review ONLY the changes shown above. You may use Read/Grep/Glob to load
surrounding code for context (call sites, type definitions, referenced
helpers), but your findings must reference lines inside the diff.

Respect the project's stated conventions from `PROJECT_INSTRUCTIONS` — they
OVERRIDE generic best-practice advice. If a convention file says "no
comments," do not flag missing comments. If it says "never use `any`," flag
every `any` in the diff even though generic advice would shrug at it.

Every finding must include: severity tier, `file:line`, what the problem
is, why it matters, and a concrete fix. No hand-wavy "consider improving"
findings. If you can't write the fix, drop the finding.

Severity tiers:
- **Critical**: ship-blocker. Will cause incidents, data loss, security
  compromise, or broken production.
- **High**: serious bug, regression risk, or important missing coverage.
  Must be addressed before merge.
- **Medium**: real issue worth fixing, but not a blocker. Reviewer should
  push back if left unfixed without rationale.
- **Low**: nit, style drift, minor improvement. Take it or leave it.
```

## Phase 4: Report Assembly

After all agents return, compile a unified report. Print it inline to the
terminal — do NOT write it to a file.

### 4.1 Deduplicate across domains

Two agents may catch the same root issue from different angles (e.g., a
silent `catch` block is both `errors` and `correctness`). Consolidate to
the most specific domain and drop the duplicate. Never repeat the same
`file:line` across domains.

### 4.2 Apply threshold

Filter per `THRESHOLD`:
- `critical` — Critical only
- `high` — Critical + High
- `medium` — Critical + High + Medium
- `low` (default) — all tiers

### 4.3 Print the report

```markdown
# Code Review — {scope summary}

**Scope**: {SCOPE_MODE} ({SCOPE_REF})
**Path**: {SCOPE_PATH or "repo-wide"}
**Files changed**: {count}
**Lines changed**: +{added} / -{removed}
**Languages**: {detected}
**Domains reviewed**: {DOMAINS}
**Threshold**: {THRESHOLD}

> **Note**: This review is static analysis by Claude. Findings are
> best-effort — validate with the project's test suite and your own
> judgment before acting.

## Verdict

{BLOCK | APPROVE WITH FIXES | APPROVE}

Rules:
- Any Critical finding → BLOCK
- Any High finding (no Critical) → APPROVE WITH FIXES
- Medium/Low only → APPROVE

One-sentence justification.

## Summary

| Domain            | Critical | High | Medium | Low |
|-------------------|----------|------|--------|-----|
| Correctness       | N | N | N | N |
| Security          | N | N | N | N |
| Maintainability   | N | N | N | N |
| API & Contract    | N | N | N | N |
| Testing           | N | N | N | N |
| Errors & Resilience | N | N | N | N |
| Tech Debt         | N | N | N | N |
| **Total**         | **N** | **N** | **N** | **N** |

## Critical Findings

### Correctness — {count}
<findings>

### Security — {count}
<findings>

...one section per domain that has Critical findings...

## High Findings
(same structure)

## Medium Findings
(only if THRESHOLD includes medium)

## Low Findings
(only if THRESHOLD includes low)

## Positive Observations
<Short list — 2-5 bullets — of things the change does well. Calibrates the
report and signals what to preserve on iteration.>
```

Each finding looks like:

```markdown
#### `path/to/file.ext:142` — short title

- **What**: one-sentence description of the problem
- **Why it matters**: concrete consequence (bug class, user impact, risk)
- **Evidence**:
  ```<lang>
  <3-8 lines of the actual code>
  ```
- **Fix**: concrete code-level change. Include a snippet if it's short.
- **Confidence**: high | medium | low — and why if not high
```

If a domain or tier has no findings, write "No findings." rather than an
empty table.

## Phase 5: Fix Offer

If `FAST_FIX` is false and there is at least one Critical, High, or Medium
finding, use `AskUserQuestion`:

> "Review found **{C} critical**, **{H} high**, and **{M} medium**
> findings. Want me to apply fixes?"

Options:

1. **All actionable** — Critical + High + Medium
2. **Critical + High** — ship-blockers only
3. **Critical only** — the must-fix tier
4. **Pick** — list findings with numbers and let the user select
5. **None** — keep the report as-is

If `FAST_FIX` is true, skip straight to option 1 by default but let the
user redirect via the same menu.

### If the user chooses to apply fixes

1. Collect the selected findings.
2. Group them by file.
3. Launch parallel **write-capable** agents — one per file or small group
   (cap 6 concurrent). Each receives:
   - Target file path(s)
   - The specific findings with their suggested fixes
   - Instructions to `Read`, then `Edit` in place
   - A rule: preserve public API signatures unless a finding explicitly
     demands a signature change. If unavoidable, skip and flag.
   - A rule: one logical fix per Edit, do not opportunistically refactor.

4. After fix agents return, run the project's test command. Detect via
   manifest:
   - `package.json` + `test` script → `pnpm test` (or `npm test`)
   - `Cargo.toml` → `cargo test`
   - `pyproject.toml` / `pytest.ini` / `setup.py` → `pytest`
   - `go.mod` → `go test ./...`

   If tests fail, **do not commit and do not stage**. Print the failure,
   list applied fixes, and ask whether to revert or keep-and-debug.

5. On success, print a summary:

```markdown
## Fixes Applied

| Finding | Location | Domain | Change |
|---------|----------|--------|--------|
| <title> | file:line | correctness | replaced unchecked index with bounds check |
```

List any skipped fixes under "Skipped — Needs Manual Review" with the
reason.

### Commit discipline

Do NOT auto-commit. Leave fixes staged (or unstaged, matching the
original scope) so the user can review with `git diff` and commit
themselves. This skill produces review artifacts, not commits — the
user-controlled commit flow (via `/commit` or their normal discipline)
takes it from here.

## Safety Rails

- **Read-only through Phase 4.** Phases 1-4 never write files.
- **Respect project instructions.** If `CLAUDE.md`/`AGENTS.md` conflicts
  with generic best practices, the project instructions win.
- **No duplicate findings.** Deduplicate aggressively across domains.
- **Every finding is actionable.** Drop anything without a concrete fix
  and a real consequence.
- **Cite evidence.** Every finding has a `file:line`. A finding without a
  location is a bug in the report.
- **Preserve semantics.** Fix agents must not silently change observable
  behavior. If a fix requires a semantic change (sync → async, reordered
  side effects, error-type changes, loss of ordering guarantees), flag it
  in the report and require explicit user approval before applying.
- **Don't invent bugs.** If a suspicious pattern might be intentional,
  label the finding `Confidence: low` and explain the uncertainty rather
  than asserting it's broken.
- **Diff-scoped only.** Agents may read surrounding code for context but
  must not report findings on lines outside the diff.

## When Not to Use This Skill

- **Whole-project audits** → use `/security-hardening`, `/optimize`, or
  `/complexity-audit` instead.
- **Single-dimension review** (e.g., only bugs) → invoke the
  `critical-reviewer` agent directly.
- **Post-merge retrospective** → check out the merge commit and use
  `branch` mode, or pass a commit range.
- **Pre-refactor exploration** → use `code-smell-detector` agent for
  structural smells, not line-level review.
