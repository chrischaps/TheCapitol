---
name: next-issue
description: Pick up and implement the next open issue for The Capitol's Flint client (or a specific issue number). Use when asked to "tackle the next issue", "do the next one", "work on #N", or to continue the Flint client work. Finds the next unblocked issue on GitHub, loads the plan context, works in its own worktree, verifies against the issue's acceptance list, then commits, pushes and closes.
---

# next-issue

The work queue for The Capitol's Flint client is the GitHub issues labelled `flint-client` on `chrischaps/TheCapitol`. Each issue is one work package from `the-capitol-flint-client.md`, the plan doc at the repo root. Issues were created in dependency order, so **the lowest open number is usually next**.

If the user names an issue number, skip step 1 and use that one.

## 1. Find the next issue

```bash
gh issue list --repo chrischaps/TheCapitol --label flint-client --state open --limit 100 --json number,title,labels --jq 'sort_by(.number)[] | "#\(.number) \(.title)"'
scripts/issue.sh list      # claims held by every agent, on every machine
```

Starting from the lowest number, take the first issue that meets all three conditions:
- **Not claimed.** It isn't in `scripts/issue.sh list`, and no open PR mentions it. A `claimed` label on GitHub is only a display; the claim branch is what counts.
- **Unblocked.** Every issue under its "Depends on" heading is closed. Check them with `gh issue view <dep> --json state`.
- **Doesn't need Chris first.** Some issues say they need a decision from Chris, B7 for example. Do the write-up part, post it as a comment on the issue, and stop there.

Then claim it, before reading anything else:

```bash
scripts/issue.sh claim <n>
```

It makes your worktree, `../TheCapitol-<n>` on branch `issue-<n>` (from `origin/issue-<n>` if an earlier agent pushed work in progress, otherwise `origin/master`), and prints its path. Work only there: `renew`, `release` and `land` refuse to run anywhere else.

Exit 2 means another agent got there first, perhaps seconds ago on another machine, or in another session on this same checkout. Go back to the list and take the next one. **Don't touch an existing `../TheCapitol-<n>` worktree for an issue you didn't claim**, even when it's on this machine: it's that session's (#134). If `claim` refuses because the worktree or branch already exists, the same goes: it's someone else's until `issue.sh list` shows the claim stale. Never work on an issue you didn't claim. The claim protocol is described under "Several agents, several machines" in the root `CLAUDE.md`.

Tell the user which issue you picked and why any lower-numbered ones were skipped. Then start without waiting.

## 2. Load the context

1. `gh issue view <n>`. Read the Work, Depends on, Acceptance and Verify sections.
2. Read every plan section listed under "Read first". The links point into `the-capitol-flint-client.md`; read the local copy.
3. Read `flint-client/CLAUDE.md` if it exists (it's created by P0.2). It has the layout, build commands and subtree commands.
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
- Tick off every **Acceptance** item, and be honest about any you couldn't do.
- Visual work: capture screenshots into `flint-client/renders/` (gitignored) and look at them yourself before calling it done. `flint render` can't show the runtime world, so use the client's `--offline fixtures/terrain.json` capture mode.
- Mark anything only a human can judge with **[H]**, saying what "pass" looks like.

The local stack is `docker-compose up -d`, then `cargo run` from `backend/`. It's one login session per account, so a Flint and React two-client test needs two accounts.

## 5. Keep the plan honest

If you made a decision the plan didn't, or found a size, dependency or approach was wrong:
- update `the-capitol-flint-client.md` in the same commit
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
scripts/issue.sh land <n> --verify "<the issue's Verify commands>"
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
