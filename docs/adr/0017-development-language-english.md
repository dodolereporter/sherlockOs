# ADR 0017: Development language: English

- Status: accepted
- Date: 2026-11-25
- Lot: Q-07

## Context

Until now, the repository mixed two languages. Code identifiers were in English, like the rest of
the Rust ecosystem, but comments, documentation, commit messages, folder names, `just` recipes,
command-line options and test names were in French (section "Langue" of `docs/contexte.md`).
The result was names like « desktop/compositeur », « just verifier » or
« compositeur --imbrique » next to English types and functions, and comments that switched
language from one crate to the next once new lots started writing in English.

On 25 November 2026, the author decided, and insisted (« même dans le code »), that all
development is done in English.

## Decision

Everything that belongs to development is written in English: identifiers, names of folders,
crates, binaries, files and recipes, command-line options and environment variables, code
comments and doc comments, technical documentation, commit messages and pull requests.

The interface that the user sees stays in French, addressing the user as "tu": the strings of
`sherlock_ui`, the messages shown to the user, what Sherlock says, the content of `.desktop` files.

Names approved by the author:

- applications: Evidence (files), Inquiry (terminal), Control Room (settings);
- shell programs, under technical names: compositor, bar, dock, launcher, notifications,
  sherlockd.

ADRs 0000 to 0016 stay in French, as the history of the decisions; they are not translated. The
author's design reference in `docs/design/` is not translated either. ADRs from 0017 on are
written in English.

## Consequences

- Lot Q-07 renames what was French (« desktop/compositeur » becomes `desktop/compositor`, the
  `just` recipes, the options, the scripts and folders of `distro/`, the doc pages, the tests) and
  translates every comment and technical doc, without changing behaviour.
- `CLAUDE.md` states the rule. Commit messages follow `<ID>: <what changes>`, in the present
  tense, in English.
- `distro/tests/check-language.py` (`just check-language`) reports French left in comments,
  file names and identifiers. It never reads Rust string literals, which may be interface text.
- Every lot brief, from now on, is read with this rule: a brief written in French still produces
  English code, names and docs, and a French interface.
- `docs/plan.md` and `docs/contexte.md` are still in French and still cite some old names. Whether
  to translate them is left to the author.

## What would make us reconsider

Nothing expected: the rule follows the language of the ecosystem and of the tools. A contributor
who cannot read English, or a need to publish French developer documentation, would reopen the
question for the documentation only.
