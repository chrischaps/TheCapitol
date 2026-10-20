---
name: next-issue
description: Pick up and implement the next open issue for The Capitol (any open issue, ranked bugs first, then the current plan's work packages, then follow-ups; or a specific issue number). Use when asked to "tackle the next issue", "do the next one", "work on #N", or to continue the Citizens, Economy v1, Flint client, suburb or wilds work. Finds the next unblocked issue on GitHub, loads the plan context, works in its own worktree, verifies against the issue's acceptance list, then commits, pushes and closes.
---

# next-issue

Every open issue on `chrischaps/TheCapitol` is in the queue, whatever its label. The labels say what an issue is about and what kind it is:

| Area label | Plan doc at the repo root |
|---|---|
| `citizens` (the current plan) | `the-capitol-citizens.md`: C0.1 … C3.3 |
| `economy` | `the-capitol-economy.md`: E0.1 … E4.4 |
| `flint-client` | `the-capitol-flint-client.md`: P0.1 … P8.7, B1 … B13, engine hooks H… |
| `wilds` | `the-capitol-wilds.md`: W0 … W11 |
| `suburb-v2` | `the-capitol-suburb-v2.md` |
| `infra` | none: CI, `scripts/`, agent tooling, the dev environment |

| Kind label | Meaning |
|---|---|
| a phase (`cz:C*`, `ec:E*`, `fc:P*`, `fc:backend-prereq`) | a work package from its plan, whose header line links the plan |
| `bug` | something broken: a follow-up, a regression, a limitation that costs players or agents |
| `engine-upstream` | the fix goes to Flint as a PR (see below) |
| `needs-decision` | waiting on Chris. Write up the options, comment, and stop |

An issue can carry more than one area: economy issues that touch the client carry `flint-client` too, and still belong to the economy plan. Below, "the plan" means the doc the issue's header links, if it has one. A follow-up with no plan link takes its context from the issue it came from.

If the user names an issue number, skip step 1 and use that one.

## 1. Find the next issue

```bash
gh issue list --repo chrischaps/TheCapitol --state open --limit 200 --json number,title,labels \
  --jq 'sort_by(.number)[] | "#\(.number) [\([.labels[].name] | join(","))] \(.title)"'
scripts/issue.sh list      # claims held by every agent, on every machine
```

**Skip** an issue if any of these holds:
- **It's claimed.** It's in `scripts/issue.sh list`, or an open PR mentions it. A `claimed` label on GitHub is only a display; the claim branch is what counts.
- **It's blocked.** An issue under its "Depends on" heading is still open. Check with `gh issue view <dep> --json state`.
- **It waits on Chris.** It's labelled `needs-decision`, or its text says it needs his decision or confirmation first (★ defaults, "[needs Chris's decision]", "Chris approves"). If nobody has written up the options yet, do that, post it as a comment, and stop there. Don't build it. If the comments show Chris has answered, it's free: take the label off (`gh issue edit <n> --remove-label needs-decision`) and go on.

**Rank** what's left in three tiers. Take the lowest number in the highest tier that has one:
1. **Bugs that hurt.** They lose players' items or Strands, crash the client on a normal path, or break CI or `land` for every agent. Judge by the text, not just the `bug` label.
2. **The current plan's work packages.** Issues with a phase label of the current plan (`cz:*` and #211, while the root `CLAUDE.md` says Citizens is the focus), in dependency order.
3. **Everything else:** other plans' leftovers, follow-ups, limitations, polish. Lowest number first, whatever the area.

Chris can always name an issue or a tier ("do the next client one"); that wins. If an issue has no area label, or no kind label, add the right ones while you're there (`gh issue edit <n> --add-label ...`).

Then claim it, before reading anything else:

```bash
scripts/issue.sh claim <n>
```

It makes your worktree, `../TheCapitol-<n>` on branch `issue-<n>` (from `origin/issue-<n>` if an earlier agent pushed work in progress, otherwise `origin/master`), and prints its path. Work only there: `renew`, `release` and `land` refuse to run anywhere else.

Exit 2 means another agent got there first, perhaps seconds ago on another machine, or in another session on this same checkout. Go back to the list and take the next one. **Don't touch an existing `../TheCapitol-<n>` worktree for an issue you didn't claim**, even when it's on this machine: it's that session's (#134). If `claim` refuses because the worktree or branch already exists, the same goes: it's someone else's until `issue.sh list` shows the claim stale. Never work on an issue you didn't claim. If Chris tells you to take over a claim he knows is dead, use `scripts/issue.sh claim <n> --steal --override "<his reason>"`, and only then. Before stealing, save any work in the old worktree (see "Claims go stale" in the root `CLAUDE.md`). The claim protocol is described under "Several agents, several machines" in the root `CLAUDE.md`.

Tell the user which issue you picked, its tier, and why any lower-numbered ones were skipped. Then start without waiting.

## 2. Load the context

1. `gh issue view <n>`. Read the Work, Depends on, Acceptance and Verify sections.
2. Read every plan section listed under "Read first". The links point into the plan doc (see the area table above); read the local copy. A follow-up without them: read the issue it came from, and the plan section that issue named.
3. If the issue touches `flint-client/`, read `flint-client/CLAUDE.md`. It has the layout, build commands and subtree commands.
4. Skim the closing comments of the issues this one depends on (`gh issue view <dep> --comments`). They record what was actually built, and that can differ from the plan.
5. Check Claude's Vault for recent Capitol and Flint notes: `ls ~/OneDrive/Documents/"Claude's Vault" | grep -i -E "capitol|flint" | tail`.
6. Read the code before changing it. The backend contract is in `backend/src`, and the plan cites file:line for most of it. Line numbers drift, so search when they don't match.

## 3. Work in your own worktree

Another agent, or Chris, may be working at the same time. `claim` already made your worktree, `../TheCapitol-<n>`. Run everything for the issue from there, including `issue.sh renew`, `release` and `land`.

If it started from `origin/issue-<n>` (the claim says so), an earlier agent pushed work in progress before its claim went stale. Read its commits first.

If the work will outlast a day (an engine PR waiting for Chris, say), run `scripts/issue.sh renew <n>` at the start of each session so the claim doesn't look abandoned. Push work in progress with `git push origin issue-<n>` so it survives if this machine goes away.

- Leave the main checkout alone, and don't touch changes you didn't make.
- Stage files by path, never with `git add -A`.
- Use your own `CARGO_TARGET_DIR` if another build or a running client could lock the exe.
- Kill only processes you started, by PID.

### Engine changes (issues labelled `engine-upstream`)

Engine changes are made in `flint-client/engine/` and go upstream to Flint (`https://github.com/chrischaps/Flint`, primary branch **`main`**):
- Push them with `git subtree push --prefix=flint-client/engine flint feature/<name>` and open a PR. Never push to Flint `main` directly.
- Chris merges Flint PRs. Stop at "PR open", say so in your summary, and leave the issue open until the merge and the follow-up `git subtree pull --prefix=flint-client/engine flint main --squash`. Keep the claim while you wait, and push `issue-<n>` so whichever agent does the follow-up can pick it up. If you stop for good without landing, run `scripts/issue.sh release <n>` and say in an issue comment where the work stands.
- Don't rebase a branch that holds the subtree pull; merge master in. `land` does this for you (#101), so let it catch up rather than rebasing by hand.
- Don't run `cargo fmt` on engine files you didn't change.
- Test crate by crate with `cargo test -p <crate>`. `--workspace` fails on Windows because of `ndk-sys`.

## 4. Verify

- Run the commands under the issue's **Verify** heading.
- Run `scripts/ci.sh`: what CI runs after you land (clippy with `-D warnings`, the tests, the database tests, and the Flint client when you touched `flint-client/` or `protocol/`). An issue's Verify is usually narrower. `cargo test` passing doesn't mean clippy does: master was red for 31 pushes on dead code that only tests used (#216).
- Tick off every **Acceptance** item, and be honest about any you couldn't do.
- Visual work: capture screenshots into `flint-client/renders/` (gitignored) and look at them yourself before calling it done. `flint render` can't show the runtime world, so use the client's `--offline fixtures/terrain.json` capture mode.
- Mark anything only a human can judge with **[H]**, saying what "pass" looks like.

The local stack is `docker-compose up -d`, then `cargo run` from `backend/`. An account can sign in from several clients at once (E0.8), but only the newest plays the avatar; the others watch. So a two-player test still needs two accounts.

## 5. Keep the plan honest

If you made a decision the plan didn't, or found a size, dependency or approach was wrong:
- update the plan doc (the one the issue links) in the same commit, including an as-built note in the work package's row
- if later issues are affected, comment on them (`gh issue comment <m> --body ...`) rather than silently diverging
- if you hit a limitation and worked round it (engine, tooling, backend), cut an issue for it now (see "Cut a ticket for every limitation" in the root `CLAUDE.md`), link it from the workaround, and name it in your closing comment

## 6. Commit, push, close

```bash
git add <files you changed>
git diff --cached --stat
git commit -m "$(cat <<'EOF'
[<WP id>] <issue title>

- what was done
- what was decided

Closes #<n>

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
scripts/issue.sh land <n> --verify "scripts/ci.sh && <the issue's Verify commands>"
```

`land` catches up with `origin/master`, runs `--verify` (before the first push, and again whenever catching up changed HEAD), and pushes `HEAD:master` fast-forward only. It rebases to catch up, unless the branch holds a merge or a `git subtree` squash; then it merges master in instead, so a subtree pull keeps the merge the next `subtree pull`/`push` needs. If another agent lands first, it loops. Once the push succeeds, it releases the claim and deletes `origin/issue-<n>`. Exit 3 is a conflict: resolve it, `git rebase --continue` (or `git commit`, if it was a merge), then run `land` again; it verifies the result. Never force-push master, and never push it any other way.

The push closes the issue, because of `Closes #<n>`. Post the summary for whoever picks up the next one as a comment. Don't use `gh issue close <n> --comment`: on an issue that's already closed it fails, and the comment is never posted.

```bash
gh issue comment <n> --body "Done in <sha>.

<What was built, and anything that differs from the plan.>
Verified: <commands run>. [H] <what Chris should look at, if anything>"
gh issue view <n> --json state -q .state   # CLOSED; if not, gh issue close <n>
```

Then clean up. Run these from the main checkout, not from the worktree or its parent directory:

```bash
git worktree remove ../TheCapitol-<n>
git branch -d issue-<n>
```

Chris works in the main checkout, and nothing else updates it. Without this step his `master` falls behind and his `[H]` checks run against stale code. Fast-forward it, but only if both of these hold:
- it's on `master`
- none of the incoming commits touch a file he has modified

If either check fails, leave it alone and tell him how far behind it is.

```bash
git fetch origin
git branch --show-current                           # must print master
git status --short                                  # his local changes
git diff --name-only HEAD origin/master             # must not overlap the modified files above
git merge --ff-only origin/master
```

His untracked and modified files carry over a fast-forward untouched. Never stash, reset or check out over them.

## 7. Leave a trail

- Write a vault note with the `vault-note` skill. Lead with what the next issue's agent should know.
- In your final message to the user, give:
  - the issue number and title
  - what changed
  - the verification results
  - any [H] items
  - the next issue in line
