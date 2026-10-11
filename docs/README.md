# Documentation

User- and contributor-facing documentation that ships with the repository. Agent working notes live
under the untracked `.agents/` workspace instead — see [`.agents/docs/`](../.agents/docs/) if you have
a working tree with that directory present (it is intentionally **not** in git).

## Contents

| Path | What it is |
|---|---|
| [`reference/`](reference/) | External and historical reference material kept for context |
| [`reference/rust-scoop-reimplementation-chawyehsu.zh.md`](reference/rust-scoop-reimplementation-chawyehsu.zh.md) | Chinese translation of the upstream blog post *Reimplementing Scoop in Rust* by Chawye Hsu (2023-08-10), the origin of this project |

## Repository-level docs

These live at the repository root rather than here:

- [`README.md`](../README.md) — English user guide
- [`README-zh.md`](../README-zh.md) — Chinese user guide
- [`ARCHITECTURE.md`](../ARCHITECTURE.md) — module layout and data flow
- [`CHANGELOG.md`](../CHANGELOG.md) — released changes (Keep a Changelog)
- [`AGENTS.md`](../AGENTS.md) — agent/contributor working conventions

## Rules

1. **Everything here is tracked.** Anything under `docs/` must make sense to someone who cloned the
   repository, so it must not link into the untracked `.agents/` workspace as an authority.
2. **English first.** Documents aimed at users are written in English with a `.zh.md` counterpart when
   a Chinese version exists, mirroring `README.md` / `README-zh.md`.
3. Filenames are lowercase kebab-case; a Chinese counterpart keeps the `.zh.md` suffix.
