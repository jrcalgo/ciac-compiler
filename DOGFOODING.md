# Running a dogfooding session

This is the script for the maintainer running the first outside
session of CIaC. It exists because `29UpdatePlan.md`'s own three
author-run transcripts (`docs/dogfooding/transcripts/`) can only ever
measure *mechanical* friction — broken commands, missing
prerequisites, misleading output, slow steps. An author already knows
what the tool is for and what the guides mean; only a real stranger's
hour surfaces *conceptual* friction — the confusion the author can't
predict because they're not confused. This document's exit criterion
is blunt: **you should be able to hand this file and a laptop to a
colleague tomorrow and run the session without preparing anything
else.**

## Who to recruit

A backend developer who has never seen CIaC. Not a friend of the
project, not someone who has watched a demo — someone who will read
the README the way a stranger does. Comfort with a terminal is enough;
no Rust knowledge is needed, and none of the session touches CIaC's
own source.

Send them a one-line heads-up beforehand so the block of time is real:

> "I'd like an hour of your time to try a tool I've been building. No
> prep needed — just bring a laptop and don't look anything up ahead
> of time. I'll mostly watch and take notes; ask me anything you'd
> normally ask a teammate, and I'll tell you afterward what I could
> and couldn't answer in the moment."

## Setup (before they arrive)

- A machine or fresh VM with nothing pre-installed beyond a shell and
  a browser. **Do not** pre-install Rust, Docker, or any language
  toolchain — the cold-start phase is measuring exactly that gap.
- Give them: the repository URL. Nothing else — no README excerpt, no
  verbal summary of what the tool does.
- Set aside 60–90 minutes. You observe; you do not help unless they
  are fully stuck for more than a minute or two (see "Rules for the
  observer" below).
- Have this file's "Capturing" section's feedback-log template ready
  to fill in live, or copy `docs/dogfooding/feedback-log-template.md`
  to a new file before the session starts.

## The session

**Phase 1 (0:00–0:20) — cold start.** Say only: "Get this installed
and make it do something." No further instruction. OBSERVE: where do
they land first (README? a guide? somewhere else?), what do they read
in full, what do they skim or skip, what's the first command they
actually type.

**Phase 2 (0:20–0:50) — guided build.** Hand them
[Guide 01](docs/guide/01-first-service.md), then
[Guide 02](docs/guide/02-records-and-crud.md). OBSERVE: every place
they stop, re-read a paragraph, or type something the guide didn't
tell them to type — that's a gap between what the guide assumes and
what a newcomer actually knows.

**Phase 3 (0:50–1:10) — the hook.** Say only: "Make a request fail on
its third attempt and prove what happened." (This is what
[Guide 05](docs/guide/05-simulation.md), simulation, is for — don't
name it.) OBSERVE: do they find `ciac sim` on their own; once there,
does the scenario file format make sense unprompted, or do they need
the guide's exact wording before it clicks.

**Debrief (1:10– ) — ask these five questions, in order, and write
down the answers close to verbatim:**

1. What is this tool, in your words?
2. What almost made you quit?
3. What surprised you, good or bad?
4. Would you use it? For what? Why not?
5. What did you want that wasn't there?

## Rules for the observer

Stay silent except at phase transitions. Every time you feel the urge
to jump in and explain something, don't — instead, write down *what*
needed explaining. That urge is the data; explaining it away destroys
the very friction you're there to measure. The one exception: if
they've been stuck on the same step for more than two or three
minutes with no progress, you may nudge them back on track (note that
you did, and where) rather than burn the whole session on a dead end.

## Capturing

One observation per line, tagged `{friction|concept|bug|want}`:

- `friction` — a mechanical stumble: a command that didn't work, a
  missing prerequisite, output that misled them, a step that was
  slower than it should be. (The same category the author transcripts
  measure — a session finding one here means the transcripts missed
  it, which is itself worth noting.)
- `concept` — they understood the mechanics but not the idea: what a
  capability is, why a pipeline looks the way it does, what
  simulation is actually for. This is the category only a real
  stranger can supply.
- `bug` — CIaC did something outright wrong (crashed, generated code
  that doesn't compile/run, produced an incorrect result).
- `want` — a capability or workflow they reached for that isn't there.

A blank template lives at
[`docs/dogfooding/feedback-log-template.md`](docs/dogfooding/feedback-log-template.md);
copy it per session (e.g. `docs/dogfooding/sessions/2026-08-01.md`)
rather than editing it in place, so each session's raw log is kept.

### Filing what you find

File one issue per line item that's actionable on its own — don't
batch unrelated findings into one issue.

| Tag | Template | Notes |
| --- | --- | --- |
| `friction` | [Docs friction](.github/ISSUE_TEMPLATE/docs_friction.md) if it's about a doc/guide reading experience; [Bug report](.github/ISSUE_TEMPLATE/bug_report.md) if it's a CLI/compiler behavior | |
| `concept` | [Docs friction](.github/ISSUE_TEMPLATE/docs_friction.md) | conceptual gaps are a docs problem even when no doc was literally wrong |
| `bug` | [Bug report](.github/ISSUE_TEMPLATE/bug_report.md) | |
| `want` | [Feature request](.github/ISSUE_TEMPLATE/feature_request.md) | |

Add the `dogfooding` label to every issue filed from a session — it's
how findings from a real outside tester stay distinguishable from
everything else in the tracker, and each issue template links back
here for that reason.

## After the session

Thank them — genuinely, not as a formality: they gave you the one
thing the entire arc that produced this document couldn't manufacture
on its own, a real stranger's hour. Send the filed issues their way if
they're curious what came of it; a returning tester for a follow-up
round, once the top findings are fixed, is the best second session you
can ask for.
