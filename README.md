# Kluris

> Kluris turns your AI agents into a subject matter expert that never sleeps and never quits.

*When your best engineer sleeps, Kluris doesn't. When they leave, Kluris stays.*

> 🎥 **New here?** Take the [guided tour at kluris.ngvoicu.dev](https://kluris.ngvoicu.dev/presentation.html).

```sh
curl -fsSL https://kluris.ngvoicu.dev/install.sh | sh     # macOS, Linux
```

```powershell
irm https://kluris.ngvoicu.dev/install.ps1 | iex          # Windows PowerShell
```

## What is Kluris?

Kluris gives every AI coding agent on your team the same shared knowledge:
architecture, decisions, conventions, learnings. Your agents work like a
colleague who knows every project, not a generic assistant starting from
scratch in each chat.

The knowledge lives in a **brain**: a git repository of plain markdown that
agents read, search and write. Agent and human curate it together. The agent
proposes what to write down; a human reviews and approves every piece.

Kluris itself is one small binary. It creates and maintains brains, searches
them, and installs each brain into your agents as a skill named
`kluris-<brain>`.

## Why it saves tokens

A brain is a summary your agent reads instead of the raw files. Without one,
the agent crawls the repository (and every sibling repository it needs) at the
start of every chat. With one, it loads a compact snapshot and goes straight to
the note it needs.

**Without Kluris**, an illustrative cold start on a medium repository:

```text
tree + README + CLAUDE.md         ~3,000 tokens
grep for related symbols          ~2,000 tokens
read 4-8 relevant files           ~15,000 tokens
read 2-3 sibling-project files    ~8,000 tokens
                                  ─────────────
                                  ~28,000 tokens just to orient
```

**With Kluris**, the same task:

```text
kluris wake-up --json snapshot    ~1,200 tokens (brain.md + lobes + recent + glossary)
kluris search "<query>" --json    ~400 tokens   (ranked hits with snippets)
read 1-2 matching neurons         ~1,500 tokens
                                  ─────────────
                                  ~3,100 tokens, about 9x less spent on orientation
```

These are rough estimates, not a benchmark; the numbers depend on the
repository, the agent and the prompt. The shape holds: a targeted lookup
replaces a wide crawl, and the saving grows with every project you touch.

### Why not a wiki, Notion or CLAUDE.md?

- **Wikis and Notion** are written for people. Agents can't search across them
  or write back. A brain is markdown in git, made for agents and readable by
  people.
- **CLAUDE.md** belongs to one project and one tool. A brain sits above all
  your projects and works with every agent on the team.
- **Agent memory** is controlled by the agent. A brain is curated by people:
  you decide what goes in, review every entry and correct what is wrong.

## Install

Kluris is a single binary. It needs no administrator rights and nothing else
installed; `git` is the only other tool it uses.

**macOS and Linux:**

```sh
curl -fsSL https://kluris.ngvoicu.dev/install.sh | sh
```

**Windows (PowerShell):**

```powershell
irm https://kluris.ngvoicu.dev/install.ps1 | iex
```

The installer puts `kluris` in `~/.kluris/bin` (`%USERPROFILE%\.kluris\bin` on
Windows) and adds that folder to your PATH: your shell's startup files on
macOS and Linux, your user PATH on Windows. **Open a new terminal**, then run:

```sh
kluris doctor
```

To install a particular release, set `KLURIS_VERSION`:

```sh
curl -fsSL https://kluris.ngvoicu.dev/install.sh | KLURIS_VERSION=3.0.0 sh
```

```powershell
$env:KLURIS_VERSION = '3.0.0'; irm https://kluris.ngvoicu.dev/install.ps1 | iex
```

Set `KLURIS_NO_MODIFY_PATH=1` the same way to leave your PATH and shell files
alone.

**Platforms:** macOS (Apple Silicon and Intel), Linux x86_64 and arm64 (with
static musl builds, which the installer picks when the system's glibc is too
old), Windows x86_64 and arm64.

### Update

```sh
kluris self-update            # install the newest release, then run kluris doctor
kluris self-update --check    # only show the installed and the newest version
kluris self-update --version 3.0.0
```

`self-update` checks the download's SHA-256 before it replaces the binary.
Behind GitHub's rate limit for anonymous requests, set `KLURIS_GITHUB_TOKEN`
to a GitHub token.

### Uninstall

```sh
kluris self-uninstall
```

This stops and removes the sync services (`kluris sync --install-service`),
then takes out the binary, the PATH setup and the installer's receipt. Your
config, the companion playbooks, the agent skills and every brain stay.
`kluris self-uninstall --purge` also deletes `~/.kluris` and every agent skill
Kluris wrote, after asking (`--yes` skips the question). Brains are never
touched.

### Verify a download

Every archive on the [releases page](https://github.com/ngvoicu/kluris/releases)
has a `.sha256` file beside it, the release has one `sha256.sum`, and every
artifact carries a GitHub attestation:

```sh
shasum -a 256 -c kluris-aarch64-apple-darwin.tar.gz.sha256   # or sha256sum -c
gh attestation verify kluris-aarch64-apple-darwin.tar.gz --repo ngvoicu/kluris
```

The macOS and Linux installer checks the SHA-256 for you.

## Quick start

Kluris lives in two places:

| Where | What you type | What it is |
|-------|---------------|------------|
| **Your terminal** | `kluris …` | The CLI: create, register, maintain and inspect brains |
| **Your AI agent** | `/kluris-<brain> …` | The brain's skill: the agent searches, learns and remembers through the CLI |

**1. Create a brain, in your terminal:**

```sh
kluris create            # a short wizard: name, description, location, type, git, companions
```

or non-interactively:

```sh
mkdir -p ~/brains
kluris create acme --description "Acme platform knowledge" --path ~/brains
```

```text
Brain created: acme (product-group)
  Path: /Users/you/brains/acme
  Lobes: 3

Agent skill installed: kluris-acme
  Claude/Codex/Gemini/etc: /kluris-acme
  Hermes: /skill kluris-acme  (or start with `hermes -s kluris-acme`)
  Created a new Hermes profile later? Run `kluris doctor` to refresh Kluris skills.
```

**Or join your team's brain.** A brain is a plain git repository: clone it and
register the folder (Kluris uses it in place, it copies nothing):

```sh
git clone git@github.com:acme/acme-brain.git ~/brains/acme
kluris register ~/brains/acme
```

**2. Use it, inside your AI agent**, with any project open:

```text
> /kluris-acme learn everything about this service
```

The agent studies the code and walks you through each finding. You see every
note before it is written, and you approve, edit or skip it.

**After an update**, `kluris doctor` checks your setup and rewrites every
brain's skill (`self-update` runs it for you).

## Agents

Every brain is installed as one skill, `kluris-<brain>`, a `SKILL.md` with
instructions and the `kluris` commands to run. Kluris writes it to:

| Folder | Read by |
|--------|---------|
| `~/.agents/skills/` | Cursor, Codex, Gemini CLI, Devin, OpenCode, Kilo Code, Junie and every other agent that reads this shared folder |
| `~/.claude/skills/` | Claude Code |
| `~/.hermes/skills/` and every `~/.hermes/profiles/*/skills/` | Hermes Agent |
| `~/.gemini/config/skills/` | Antigravity |
| `$COPILOT_HOME/skills/` (when `COPILOT_HOME` is set) | GitHub Copilot |

How to call it:

- **Most agents:** `/kluris-acme`, then what you want (`/kluris-acme search auth flow`).
- **Hermes:** `/skill kluris-acme` inside a session, or start with `hermes -s kluris-acme`.

The skill is plain instructions plus shell commands, so any agent that loads
`SKILL.md` skills and can run a shell can use it. Created a new Hermes profile,
or set `COPILOT_HOME`, after adding a brain? Run `kluris doctor` to install the
skills there too.

## Working with a brain

Everything in this section happens **inside your AI agent**. The patterns are
free-form: say it naturally.

### Onboard a project

Point the project at the brain, so every agent that opens the repository finds
it:

```text
> /kluris-acme setup this project
```

The agent adds a short `## Knowledge base` section to the project's
`CLAUDE.md` and `AGENTS.md` (creating them if needed), each change approved by
you:

```markdown
## Knowledge base

Read and write to the **acme** brain through kluris (never edit
brain files by hand). Use the `/kluris-acme` skill — search,
learn, remember, create.
```

Then have it learn the project:

```text
> /kluris-acme learn the API endpoints and data model
> /kluris-acme learn the Docker and deployment setup
```

### Remember decisions and knowledge

```text
> /kluris-acme remember we chose raw SQL over an ORM for the reporting queries
> /kluris-acme remember all timestamps must be TIMESTAMPTZ
> /kluris-acme create a decision record about the auth architecture
> /kluris-acme create an incident report for the January outage
> /kluris-acme remember from this session
```

`remember from this session` goes back over the whole chat and proposes a note
for everything worth keeping, one at a time. Use it at the end of a working
session.

Before it writes, the agent syncs the brain with your team; after you approve,
it commits exactly the files it wrote with `kluris commit` and pushes them with
`kluris sync`. If a teammate changed the same thing, it shows you both
versions, asks which to keep (or proposes a merge), and settles it with
`kluris resolve`.

### Use the brain while coding

```text
> /kluris-acme search token refresh
> /kluris-acme what do we know about the auth flow?
> /kluris-acme implement the new endpoint following our conventions
> /kluris-acme use brain knowledge and codebase and let's write a spec for OAuth sign-in
```

The agent checks the brain before it answers or writes code, and flags code
that contradicts a recorded decision. "Use brain knowledge and codebase" asks
it to ground the work in both: the team's decisions and the code as it is.

### Review the brain

```text
> /kluris-acme review this brain for gaps and stale neurons
> /kluris-acme review this brain and create more synapses
> /kluris-acme fix the broken synapses
```

### Maintain it, in your terminal

```sh
kluris dream     # regenerate the maps, fix what is safe to fix, validate links
kluris status    # lobes, neuron counts and recent commits
kluris mri       # write the brain's visual map and print its link
```

A brain is a git repository: commit, push and pull it with `git` like any
other. Run `kluris dream` before you commit structural changes so the
regenerated maps go into the same commit, or let `kluris commit --brain
<name> --paths a.md,b.md` do it: it commits just those files and the
`map.md`/`brain.md` files they change, under your git identity, and leaves
every other change in the brain as it is.

### Keep it in sync with your team

```sh
kluris sync --brain acme          # fetch, merge and push one brain
kluris sync --brain all --json    # every brain, for scripts and agents
```

For each brain, `kluris sync` fetches its branch's upstream (it never asks
for credentials), fast-forwards when the brain is behind, pushes when it is
ahead, and merges when both sides have new commits:

- Merged on their own: changes to different files; the generated `map.md`
  and `brain.md` files, rebuilt from the merged brain rather than merged line
  by line; `created` (the earlier date) and `updated` (the later one);
  `related` and `tags`, both lists joined; edits to different parts of the
  same neuron; a file one side adds in a folder the other side renamed,
  which moves into the renamed folder (renaming a folder never drops a
  teammate's file).
- Left for a person: overlapping edits, a file deleted on one side and edited
  on the other, different `parent` values, any other frontmatter field
  changed differently on both sides, a file on one side where the other has
  a directory (or a rename git cannot settle), and, when the history has
  several merge bases (both sides merged each other), any file changed on
  both sides. Nothing changes then (no file, no commit, no push) until the
  conflict is settled with `kluris resolve`, and a later sync reports the
  same open conflict.

A file with uncommitted changes is never overwritten, nor is an untracked or
ignored file where an incoming file lands: that brain is not synced until you
commit, stash or move it. The repository's `pre-merge-commit`,
`prepare-commit-msg`, `commit-msg`, `post-merge` and `pre-push` hooks run
(`--no-verify` skips `pre-merge-commit` and `commit-msg`); a merge hook may
change the message, but one that stages files into the merge stops it, with
nothing merged. The brain must be the root of its
own git repository, and git must be 2.38 or newer.

With `--json`, each brain's entry has `name`, `fetched`, `merged`, `pushed`,
`auto_merged` (neurons merged on their own), `conflicts` (each with `path`,
`kind` (`content`, `delete_edit`, `parent`, `frontmatter`, `structure` or
`ambiguous_base`) and the `base`, `ours` and `theirs` blob ids, `null` where
a side has no file there), `skipped_dirty` (uncommitted files left as they
are), `offline`, `blocked` (uncommitted files in the way) and `refused` (why
the brain was left alone: an open conflict, listed in `conflicts`, or under
`--brain all` any other reason; else `null`). The exit
code is 1 when a brain has a conflict, is blocked or was refused.

### Settle a conflict

`kluris status` shows an open conflict and its paths. Choose for each path:

```sh
kluris resolve --brain acme knowledge/auth.md --ours            # keep this brain's version
kluris resolve --brain acme knowledge/oauth.md --theirs         # keep the remote's
kluris resolve --brain acme knowledge/jwt.md --file merged.md   # keep what you merged by hand
```

`--ours` and `--theirs` keep that side's version, a deletion included; for a
`structure` conflict, that side's whole structure (its file, or its directory
and everything in it), and `--file` is refused there when a side has a
directory.
`--file` content is checked first: its frontmatter must read, and a YAML
neuron needs its `#---` block. Each call records one choice; with the last
one the merge is committed under your git identity, the brain moves to it
(uncommitted changes in the way stop it, and the choices are kept) and the
result is pushed, merging again if the remote moved meanwhile. If you
committed after the conflict was found, start over with
`kluris resolve --brain acme --abort`, which drops the open conflict and
changes nothing else, then `kluris sync`.

With `--json`, a call prints `path`, `choice`, `remaining` (the paths still
to settle), `committed`, `commit` and `pushed`; `--abort` prints `aborted`
and `dropped`. While a conflict is open, `kluris status --json` adds
`sync: {blocked: true, conflicts: [...]}` to that brain.

### Stay in sync while you work

```sh
kluris sync --watch --brain acme                # in a terminal, until Ctrl-C
kluris sync --watch --brain all --interval 60   # every brain, once a minute
kluris sync --install-service --brain acme      # in the background, from login on
kluris sync --uninstall-service --brain acme
```

`kluris sync --watch` checks every 30 seconds (`--interval`, at least 5)
whether the remote's branch moved (`git ls-remote`, which never asks for
credentials) or you made a commit, and when either did it runs one
`kluris sync` for that brain, with the same rules, and prints one line
(`--json`: the document `kluris sync --json` prints, one per line). A
conflict, a block or a refusal is printed once; each tick that brain is
looked at again (one `git status`), and it is synced as soon as its branch
or the remote moves (`kluris resolve`, your commit, a teammate's push) or
the cause clears without a commit (an edit undone, a rebase aborted). As
some causes clear without anything `git status` shows (an ignored file
deleted, a `pre-push` hook repaired), it is also synced again a minute
later, then after 2, 4 and 8 minutes and every 10 minutes from then on;
the wait starts over when a head moves or the local state changes.
Offline it stays quiet and picks up when the remote answers again. Ctrl-C
lets a sync in progress finish, then exits with 0.

`--install-service` makes your system run that watch for one brain, from
login on, as you, without admin rights, and starts it:

| System | Service | Log |
|--------|---------|-----|
| macOS | launchd agent `~/Library/LaunchAgents/dev.ngvoicu.kluris.sync.<brain>.plist` | `~/Library/Logs/kluris/sync-<brain>.log` |
| Linux | systemd user unit `~/.config/systemd/user/kluris-sync-<brain>.service` (`$XDG_CONFIG_HOME` when set) | `journalctl --user -u kluris-sync-<brain>` |
| Windows | Task Scheduler task `\Kluris\sync-<brain>`, at logon, hidden | `%LOCALAPPDATA%\kluris\logs\sync-<brain>.log` |

The service runs the `kluris` that installed it, by its full path, with
your `PATH` and `KLURIS_CONFIG`; run `--install-service` again after you
move Kluris. Both flags can be run again safely. On Linux it needs a
systemd user instance (on WSL, `systemd=true` in `/etc/wsl.conf`); a
service there starts with your session, unless `loginctl enable-linger`
keeps it running without one. `kluris doctor` lists the installed services
and whether they run; `kluris remove` stops and removes the service of the
brain it unregisters, and `kluris self-uninstall` all of them.

### How the agent starts a session

The first time the skill is used in a session, the agent runs
`kluris wake-up --brain acme --json` and keeps the result: the `brain.md`
text, the lobes with their neuron counts, the most recently updated neurons,
the glossary and any deprecation warnings. It refreshes the snapshot only
after the brain changes. To see what the agent sees:

```sh
kluris wake-up --brain acme
```

Before they read, `kluris wake-up` and `kluris search` bring the brain up to
date with your team, at most once a minute and silently: they fetch the
branch's upstream and, when your branch is only behind it, fast-forward it.
The fetch and the checks take at most 3 seconds; a fast-forward, once started,
always finishes, so it starts only when it is quick and safe. The read keeps
just the fetch, and your next `kluris sync` brings the changes in, when a
checkout filter would run (an attribute such as `filter=lfs` in any
`.gitattributes`, `.git/info/attributes` or your `core.attributesFile`, and a
driver for it in git config, as Git LFS sets up), when more than 200 files
changed, when a file you ignore is where an incoming file goes, when a folder
it writes in is read-only, when the disk is nearly full, or on Windows when a
path is too long for it. A brain with local edits, a merge or rebase in
progress, an open sync conflict, a branch that diverged or no upstream is read
as it is, and a brain inside another project's repository is never fetched.
Turn it off with `KLURIS_NO_AUTO_SYNC=1` or, for one brain,
`sync: {auto_fetch: false}` in its `kluris.yml`.

## Several brains

Each brain has its own skill (`/kluris-acme`, `/kluris-personal`), so the agent
always knows which brain you mean. In the terminal:

- With one brain registered, commands use it.
- With several, pass `--brain NAME`. `dream`, `status`, `mri`, `sync` and
  the `companion` commands also take `--brain all`.
- Without `--brain`, an interactive terminal shows a numbered list of the
  brains to pick from, plus `all` where the command takes it.
- With `--json`, without a terminal, or with `KLURIS_NO_PROMPT=1`, there is no
  picker: the command stops and lists the brains to choose from.

```text
$ KLURIS_NO_PROMPT=1 kluris status
Error: Multiple brains registered. Pass --brain NAME or 'all'. Available: acme, personal
```

## What a brain looks like

```text
acme/
├── brain.md                # the root lobes (generated)
├── glossary.md             # domain terms (you write it)
├── README.md               # how to use the brain
├── kluris.yml              # your local settings (git-ignored)
├── .gitignore              # secrets, kluris.yml, brain-mri.html
├── projects/
│   ├── map.md              # the lobe's index (generated)
│   └── billing/
│       ├── map.md
│       ├── auth-flow.md    # a neuron
│       └── payments-api.yml  # a YAML neuron
├── infrastructure/
│   └── map.md
└── knowledge/
    ├── map.md
    ├── use-clerk.md        # a decision
    └── use-auth0.md        # a deprecated decision
```

| Term | Meaning |
|------|---------|
| **Brain** | A git repository of shared team knowledge |
| **Lobe** | A folder: one area of knowledge |
| **Neuron** | One markdown note |
| **Synapse** | A link between two neurons, kept in both directions |
| **Map** | A lobe's `map.md`, generated by `kluris dream` |
| **Dream** | Maintenance: regenerate maps, fix what is safe, validate links |
| **MRI** | The brain's interactive visual map |

### Neurons

A neuron is markdown with YAML frontmatter. `parent`, `created` and `updated`
are required; `related` holds the synapses, as paths relative to the neuron:

```markdown
---
parent: ./map.md
related:
  - ../../knowledge/use-clerk.md
tags: [auth, jwt]
created: 2026-09-01
updated: 2026-10-26
---

# Auth flow

Billing trusts the session [JWT](../../glossary.md#jwt) issued by Clerk.

## Token refresh

The web app refreshes the access token five minutes before it expires.
```

`kluris dream` keeps the brain consistent. It regenerates every `map.md` and
`brain.md`, refreshes `updated` dates from git history, fills in a missing
`parent`, adds the missing half of a one-way synapse, lists unlisted neurons in
their lobe's map, then reports what it could not fix: broken synapses, orphans,
frontmatter problems.

### YAML neurons

OpenAPI files, JSON Schemas and other machine-readable specs can be neurons
too. A `.yml` or `.yaml` file counts as a neuron only when it starts with a
`#---` comment block; any other YAML file in the brain is left alone. Only
`updated` is required:

```yaml
#---
# parent: ./map.md
# tags: [api, openapi]
# title: Payments API
# updated: 2026-10-01
#---
openapi: 3.1.0
```

### Deprecating a decision

When a decision is superseded, mark the old neuron instead of deleting it; the
history is worth keeping:

```yaml
---
status: deprecated
deprecated_at: 2026-09-01
replaced_by: ./use-clerk.md
---
```

Search still finds it, labelled deprecated and ranked lower, with its
replacement. `kluris dream` warns (without failing) about four problems:

- `active_links_to_deprecated`: an active neuron links to a deprecated one.
- `deprecated_without_replacement`: a deprecated neuron has no `replaced_by`.
- `replaced_by_missing`: `replaced_by` points at a file that doesn't exist.
- `replaced_by_not_active`: `replaced_by` points at something that is not an
  active neuron.

`kluris wake-up` gives agents the count, `kluris dream --json` the full list.

### Brain types

The type only decides the starting lobes. Every brain works the same
afterwards, and you can add or rename lobes at any time.

| Type | Starting lobes | For |
|------|----------------|-----|
| `product-group` (default) | `projects/`, `infrastructure/`, `knowledge/` | A group of services that share knowledge |
| `personal` | `projects/`, `tasks/`, `notes/` | One developer's projects, tasks and notes |
| `product` | `prd/`, `features/`, `ux/`, `analytics/`, `competitors/`, `decisions/` | Product management |
| `research` | `literature/`, `experiments/`, `findings/`, `datasets/`, `tools/`, `questions/` | Research projects |
| `blank` | none | Your own structure |

## Search

`kluris search` ranks neurons, glossary entries and `brain.md` by relevance:

- **Ranked**, not filtered: titles, tags, paths, headings and body text all
  count, and a match in a title or heading weighs more than one in the body.
- **Section-aware:** each `##` section is scored on its own, so a hit points at
  the heading and line where it is.
- **Forgiving:** accents don't matter (`pretul` finds `prețul`), word forms
  don't matter (`refreshing tokens` finds "refreshes the access token"), and a
  word of three or more letters also matches longer words it starts
  (`pay` finds `payments`).
- **English and Romanian.** A brain is searched in its language: set it with
  `search: language: en` or `ro` in the brain's `kluris.yml`, or let Kluris
  detect it.
- **Deprecated neurons** stay in the results, ranked lower and labelled.

```sh
kluris search "token refresh"
kluris search "raw sql" --lobe knowledge
kluris search "auth" --tag decision --limit 5
```

```text
2 result(s) for 'token refresh' in acme

  371  Auth flow
        projects/billing/auth-flow.md
        The web app refreshes the access token five minutes before it expires.

   87  JWT
        glossary.md
        JSON Web Token: the signed session token the web app sends to every API
```

With `--json`, each result also says where and how it matched: the `heading`
and `line` of the best section, the `matched_terms`, the `coverage` (the share
of the query's words it holds), the neuron's `related` paths, its `updated`
date, and `deprecated` (with `replaced_by` for a deprecated neuron):

```json
{
  "file": "projects/billing/auth-flow.md",
  "file_type": "markdown",
  "title": "Auth flow",
  "matched_fields": ["heading", "body"],
  "snippet": "The web app refreshes the access token five minutes before it expires.",
  "score": 186,
  "deprecated": false,
  "heading": "Token refresh",
  "line": 14,
  "matched_terms": ["refreshing", "tokens"],
  "coverage": 1.0,
  "related": ["knowledge/use-clerk.md"],
  "updated": "2026-10-26"
}
```

## The MRI

`kluris mri` writes the brain as one self-contained HTML page, an interactive
architecture map, and prints a `file://` link to open it:

```text
MRI complete — /Users/you/brains/acme/brain-mri.html
  Open: file:///Users/you/brains/acme/brain-mri.html
  10 nodes, 27 edges
```

- It opens on the brain's lobes and glossary. Click a folder to go into it:
  lobe, sub-lobe, neurons, as deep as your folders go. The breadcrumb and the
  back and forward buttons take you back up.
- Each card shows its kind, a title and its counts; synapses between cards are
  drawn as edges with a count.
- Click a neuron to read it. Drag to pan, scroll to zoom.
- The left panel is the file tree; the right panel searches neurons, paths,
  lobes, tags and YAML (press `/`) and lists recent changes.

![kluris mri: the root view of a brain, its lobes and glossary as boxes joined by synapse counts, with the file tree on the left and search and recent changes on the right](https://kluris.ngvoicu.dev/mri-overview.png)

The page is `brain-mri.html` in the brain folder (git-ignored); `--output`
writes it elsewhere. It needs no server and makes no network calls: open it
from disk or send it to a colleague.

## Commands

| Command | What it does |
|---------|--------------|
| `kluris create [NAME]` | Create a brain: a wizard, or flags (`--description`, `--path`, `--type`, `--remote`, `--branch`, `--no-git`, `--from-config`) |
| `kluris register <path>` | Register a brain folder already on disk (for example a `git clone`) |
| `kluris list` | List the registered brains |
| `kluris status` | Lobes, neuron counts and recent commits (`--brain`) |
| `kluris search <query>` | Ranked search (`--brain`, `--lobe`, `--tag`, `--limit`) |
| `kluris wake-up` | The compact snapshot agents load at the start of a session (`--brain`) |
| `kluris dream` | Regenerate maps, fix what is safe to fix, validate links (`--brain`, `--broken-only` with `--json`) |
| `kluris mri` | Write the brain's visual map and print its link (`--brain`, `--output`) |
| `kluris commit` | Commit only the files you name and the maps they change (`--brain`, `--paths`, `--message`, `--no-verify`) |
| `kluris sync` | Fetch, merge and push a brain, or every brain (`--brain`, `--no-verify`); keep doing it (`--watch`, `--interval`), or in the background (`--install-service`, `--uninstall-service`) |
| `kluris resolve <path>` | Settle an open sync conflict path by path (`--ours`, `--theirs`, `--file`, `--abort`, `--no-verify`) |
| `kluris companion list` | The companion playbooks, which are installed, and which brains use them (`--brain`) |
| `kluris companion add <name>` | Turn a companion on for a brain (`--brain`) |
| `kluris companion remove <name>` | Turn a companion off for a brain (`--brain`) |
| `kluris remove <name>` | Unregister a brain and remove its sync service; its files stay on disk |
| `kluris doctor` | Check git, PATH, the config and every brain, then refresh companions and agent skills (`--no-refresh`: checks only) |
| `kluris self-update` | Update to the newest release, or `--version X` (`--check`: only look) |
| `kluris self-uninstall` | Uninstall Kluris (`--purge`, `--yes`) |
| `kluris help [COMMAND]` | The command list, or one command's summary |

Every command takes `--json` for scripts and agents, and `--help`.

## Configuration

**`~/.kluris/config.yml`** lists the registered brains. `KLURIS_CONFIG` points
Kluris at another file.

```yaml
brains:
  acme:
    path: /Users/you/brains/acme
    description: Acme platform knowledge
```

**`<brain>/kluris.yml`** holds your own settings for one brain. It is
git-ignored, so every teammate can keep different ones:

```yaml
name: acme
description: Acme platform knowledge
agents:
  commands_for:      # which agents get the kluris-acme skill
  - claude
  - cursor
  - devin
  - copilot
  - codex
  - gemini
  - hermes
  - kilocode
  - junie
  - antigravity
companions: []       # see "Specmint companions"
search:
  language: en       # en or ro; detected when absent
sync:
  auto_fetch: true   # false: wake-up and search read without fetching
```

Trim `agents.commands_for` to the agents you use and run `kluris doctor`. The
shared `~/.agents/skills/` folder always gets every brain's skill.

**Environment variables**

| Variable | Effect |
|----------|--------|
| `KLURIS_CONFIG` | Use this file instead of `~/.kluris/config.yml` |
| `KLURIS_NO_PROMPT=1` | Never prompt; commands that need a choice stop and say what to pass |
| `KLURIS_NO_AUTO_SYNC=1` | `wake-up` and `search` read without fetching the brain first |
| `COPILOT_HOME` | Also install the skills into `$COPILOT_HOME/skills` |
| `KLURIS_GITHUB_TOKEN` | A GitHub token for `self-update` |
| `KLURIS_VERSION` | The release the install scripts install |
| `KLURIS_NO_MODIFY_PATH=1` | The install scripts leave PATH and shell files alone |

## Specmint companions

[Specmint](https://specmint.ngvoicu.dev) turns a feature request into a
lasting spec through research, an interview and a written plan. Kluris ships
four Specmint playbooks as **companions** you can turn on per brain:

| Companion | What it is |
|-----------|------------|
| [`specmint-core`](https://github.com/ngvoicu/specmint-core) | Spec-first work: research, interview, spec, implement |
| [`specmint-tdd`](https://github.com/ngvoicu/specmint-tdd) | The same, with strict TDD: a failing test before any code |
| `specmint-core-html` | Spec-first, with `.specs/<id>/SPEC.html` as the spec |
| `specmint-tdd-html` | TDD, with `SPEC.html` carrying the tasks, test state, log and progress |

```sh
kluris companion add specmint-core --brain acme
kluris companion add specmint-tdd --brain all
kluris companion list
kluris companion remove specmint-core --brain acme
```

Turning one on copies its folder to `~/.kluris/companions/<name>/` and adds a
short section to the brain's skill. From then on, before multi-step work (a
new feature, a refactor across files) the agent follows the playbook: it
researches with the brain and the code, interviews you, then writes the spec.
Small single-file edits skip it. `kluris doctor` keeps the copies current.

```text
> /kluris-acme let's spec out adding OAuth sign-in with GitHub
```

## Building from source

You need Rust (the toolchain is pinned in `rust-toolchain.toml`) and git.

```sh
git clone https://github.com/ngvoicu/kluris.git
cd kluris
cargo build --release          # target/release/kluris
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

A build from source is not managed by `self-update` or `self-uninstall`;
update it by pulling and building again.

## License

MIT

---

<!-- ngvoicu author section — identical across all ngvoicu repos, keep in sync -->
## AI-native toolkit

This project is part of a larger AI-native toolkit — and of a way of working your whole team can adopt: talks (["Becoming an AI Native Company"](https://ngvoicu.dev/becoming-an-ai-native-company/)), hands-on team training that teaches employees to use AI, and [AI adoption consulting for engineering teams](https://ngvoicu.dev/#consulting).

- Site: [ngvoicu.dev](https://ngvoicu.dev)
- Contact: [office@ngvoicu.dev](mailto:office@ngvoicu.dev) · +40 734 704 910

Toolkit: [Specmint](https://specmint.ngvoicu.dev) (durable AI coding specs) · [Kluris](https://kluris.ngvoicu.dev) (team knowledge brains) · [ConsensFlow](https://consensflow.ngvoicu.dev) (cross-agent second opinions)
