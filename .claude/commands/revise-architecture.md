---
description: >
  Revise docs/ARCHITECTURE.md to reflect current codebase state. Detects
  what changed since the doc was last updated, then surgically updates
  affected sections rather than rewriting from scratch.
argument-hint: "[optional: area of focus or summary of recent changes]"
---

# Revise Architecture Documentation

You are updating `docs/ARCHITECTURE.md` to reflect the current state of the
distant codebase. This is a **surgical revision** — you update only what has
changed, preserving the existing structure and writing style.

Load the `architecture-guide` skill for document structure, conventions, and
section-to-code mappings before proceeding.

## Phase 1: Detect Drift

Determine what has changed since the architecture doc was last updated.

1. Read `docs/ARCHITECTURE.md` in full to understand current documented state
2. Find when the doc was last updated:
   ```bash
   git log --oneline -1 -- docs/ARCHITECTURE.md
   ```
3. See what code changed since then:
   ```bash
   git log --oneline <last-update-hash>..HEAD --stat
   ```
4. **If `$ARGUMENTS` is provided**, use it as the focus area instead of
   auto-detecting from git history. Still run the git commands above for
   context, but prioritize the user-specified area.
5. Cross-reference changed files against the section-to-code mapping (from
   the `architecture-guide` skill) to identify which of the 12 sections are
   potentially affected.

## Phase 2: Explore Affected Areas

For each affected section, use **`rust-explorer`** agents (up to 3 in
parallel) to investigate what changed:

- Read current code for types, traits, enums mentioned in the section
- Identify **new** types/variants/methods not yet documented
- Identify **removed or renamed** types still referenced in the doc
- Check Mermaid diagrams against actual code flow
- Verify variant counts, table contents, and code signatures

**Important:** Use `rust-explorer` agents, NOT the builtin `Explore` agent.
These agents have project-specific skills and LSP access.

## Phase 3: Change Report

Before making any edits, present a structured change report to the user:

```
== Architecture Drift Report ==

Last doc update: <commit hash> (<date>)
Commits since:   <count>

Sections Needing Updates:
  - Section N: <name> — <what changed>
  - Section M: <name> — <what changed>

Specific Changes:
  - [Added] <new type/variant/command>
  - [Removed] <deleted type/variant>
  - [Renamed] <old name> → <new name>
  - [Modified] <changed signature or behavior>

Diagrams Needing Revision:
  - Diagram in Section N: <what needs updating>

Sections Unchanged:
  - Section X: <name> — no drift detected

Proposed New Sections (if any):
  - <section name> — <justification>
```

**Wait for user confirmation before proceeding to Phase 4.**

## Phase 4: Apply Revisions

Use **`rust-coder`** to surgically edit `docs/ARCHITECTURE.md`:

- Edit **only** the sections identified in the change report
- Do NOT rewrite sections that haven't changed
- For each affected section:
  - Update variant counts and type signatures
  - Add/remove/rename types in tables
  - Update or add Mermaid diagrams as needed
  - Add entries for new CLI commands
  - Update code blocks with current signatures
- For entirely new subsystems, propose a new section with appropriate
  placement in the document structure
- Preserve the existing writing style:
  - Tables for comparisons and type listings
  - Mermaid `flowchart` for static relationships, `sequenceDiagram` for
    temporal flows
  - Rust signatures in code blocks for key types/traits
  - Protocol variants grouped by domain

**Important:** Use `rust-coder`, NOT a generic implementation agent.

## Phase 5: Validate

Use **`code-validator`** to verify the accuracy of changes:

- Cross-reference updated type names against source code
- Verify Mermaid diagram accuracy (node names, relationships)
- Check that variant counts match reality
- Verify table contents against actual code
- Max 3 rounds of fix-and-review

**Important:** Use `code-validator`, NOT a generic review agent.

## Phase 6: Summary

Report what was done:

```
== Architecture Revision Summary ==

Sections Updated:
  - Section N: <name> — <brief description of changes>

Sections Added:
  - Section N: <name> (if any)

Sections Unchanged:
  - Section X: <name>

Diagrams Modified:
  - <diagram description> in Section N

Validation: PASS (N rounds)
```

## Rules

- **Never rewrite untouched sections** — surgical edits only
- **Always use project agents**: `rust-explorer`, `rust-coder`, `code-validator`
- **Present the change report before editing** — no surprises
- **Follow the document's existing structure** (12 numbered sections + ToC)
- **If a new section is warranted**, propose it explicitly with justification
- **Update the Table of Contents** if section titles or numbering change
- **Preserve Mermaid diagram styling** (colors, subgraph labels, node formats)
