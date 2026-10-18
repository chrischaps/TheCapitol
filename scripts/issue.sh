#!/usr/bin/env bash
# Claim, renew, release and land GitHub issues for agents that share one
# GitHub account across several machines.
#
# A claim is a branch on origin, claim/issue-<n>, holding one commit with no
# files whose message names the agent. Git updates a remote ref atomically,
# so when two agents push a claim for the same issue, the second push is
# rejected as non-fast-forward and only one agent wins. Labels, assignees and
# comments can't do that: every agent is the same GitHub user, and none of
# them is compare-and-swap.
#
#   scripts/issue.sh list                     every claim: issue, holder, age
#   scripts/issue.sh claim <n>                claim issue n (exit 2 if held) and
#                                             make its worktree, ../<repo>-<n>
#   scripts/issue.sh claim <n> --steal        take over a claim older than the TTL
#   scripts/issue.sh claim <n> --no-worktree  claim without making a worktree
#   scripts/issue.sh renew <n>                refresh your claim's timestamp
#   scripts/issue.sh release <n>              give up your claim
#   scripts/issue.sh land <n> [--verify CMD]  catch up with master, verify, push,
#                                             release the claim
#
# Two sessions started in one checkout share its .git, so the record of a
# claim can't by itself say which of them made it (#134). So claim makes the
# worktree, records its path beside the agent id, and renew, release and land
# run only from that worktree. A session whose claim exits 2 gets no worktree,
# and must leave the existing one alone.
#
# Environment:
#   CLAIM_REMOTE     remote to use (default origin)
#   CLAIM_TTL_HOURS  age after which a claim may be stolen (default 24)
#   CLAIM_AGENT_ID   name to claim under (default <host>-<time>-<random>);
#                    when set, renew, release and land refuse a claim made
#                    under another name
#   CLAIM_NO_GH=1    skip GitHub calls (labels, comments, issue state); for tests

set -euo pipefail

REMOTE=${CLAIM_REMOTE:-origin}
TTL_HOURS=${CLAIM_TTL_HOURS:-24}
LABEL=claimed


die() { echo "issue.sh: $*" >&2; exit 1; }

usage() { sed -n '12,20p' "$0" | sed 's/^# \{0,1\}//'; exit 1; }

gh_on() { [ "${CLAIM_NO_GH:-}" != 1 ]; }

ref_of() { echo "refs/heads/claim/issue-$1"; }

# Local record of the claims this checkout made, shared by all its worktrees.
# Line 1 is the agent id, line 2 the worktree claim made (absent after
# --no-worktree, and in records older than #134). Only the machine that
# claimed an issue can renew, release or land it, and only from that worktree.
common_dir() { git rev-parse --path-format=absolute --git-common-dir; }
record_dir() { echo "$(common_dir)/agent-claims"; }
record_of() { echo "$(record_dir)/issue-$1"; }
record_agent() { sed -n 1p "$(record_of "$1")"; }
record_worktree() { sed -n 2p "$(record_of "$1")"; }

# ../<repo>-<n> beside the main checkout, whichever worktree we're run from.
worktree_of() {
  local main
  main=$(dirname "$(common_dir)")
  echo "$(dirname "$main")/$(basename "$main")-$1"
}

now() { date +%s; }

# Prints "<sha> <committer time>" of the remote claim, or nothing if unclaimed.
remote_claim() {
  local ref sha
  ref=$(ref_of "$1")
  sha=$(git ls-remote "$REMOTE" "$ref" | cut -f1)
  [ -n "$sha" ] || return 0
  git fetch -q "$REMOTE" "$ref" 2>/dev/null || true
  echo "$sha $(git log -1 --format=%ct "$sha")"
}

claim_body() { git log -1 --format=%B "$1"; }

agent_of() { claim_body "$1" | sed -n 's/^agent: //p'; }

age_text() {
  local s=$(( $(now) - $1 ))
  if [ "$s" -lt 3600 ]; then echo "$(( s / 60 ))m"; else echo "$(( s / 3600 ))h$(( s % 3600 / 60 ))m"; fi
}

make_claim() { # <n> <agent> [parent]
  local msg parent=()
  [ -n "${3:-}" ] && parent=(-p "$3")
  msg="claim #$1

agent: $2
host: $(hostname)
at: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf '%s\n' "$msg" | git commit-tree "$(git mktree </dev/null)" ${parent[@]+"${parent[@]}"}
}

# Fails unless this checkout holds the claim and we're in the worktree that
# claim made. Prints the remote claim sha.
require_mine() {
  local n=$1 rec agent wt here sha _t
  rec=$(record_of "$n")
  [ -f "$rec" ] || die "#$n wasn't claimed from this checkout (no $rec)"
  agent=$(record_agent "$n")
  if [ -n "${CLAIM_AGENT_ID:-}" ] && [ "$CLAIM_AGENT_ID" != "$agent" ]; then
    die "#$n was claimed here as $agent, not as $CLAIM_AGENT_ID: it's another session's claim"
  fi
  wt=$(record_worktree "$n")
  if [ -n "$wt" ]; then
    here=$(git rev-parse --show-toplevel)
    [ "$here" = "$wt" ] \
      || die "#$n belongs to the session working in $wt; run this from there, not $here. If that session isn't you, leave it alone"
  fi
  read -r sha _t <<<"$(remote_claim "$n")"
  [ -n "${sha:-}" ] || die "#$n has no claim on $REMOTE"
  [ "$(agent_of "$sha")" = "$agent" ] \
    || die "#$n is now held by $(agent_of "$sha"), not $agent"
  echo "$sha"
}

cmd_list() {
  local found=0 ref sha n
  while read -r sha ref; do
    [ -n "$sha" ] || continue
    found=1
    n=${ref##*/issue-}
    git fetch -q "$REMOTE" "$ref" 2>/dev/null || true
    printf '#%-5s %-40s %s ago\n' "$n" "$(agent_of "$sha")" "$(age_text "$(git log -1 --format=%ct "$sha")")"
  done < <(git ls-remote "$REMOTE" 'refs/heads/claim/issue-*')
  [ "$found" = 1 ] || echo "no claims"
}

held() { # <n> <claim sha> <claim time> <worktree>
  echo "#$1 is already claimed by $(agent_of "$2"), $(age_text "$3") ago. It isn't yours: pick another issue, and don't touch $4" >&2
  exit 2
}

cmd_claim() {
  local n=${1:?issue number} steal="" make_wt=1 ref agent sha old old_t state wt base="" arg
  shift
  for arg in "$@"; do
    case $arg in
      --steal) steal=--steal ;;
      --no-worktree) make_wt="" ;;
      *) die "claim: unknown option $arg" ;;
    esac
  done
  ref=$(ref_of "$n")
  if gh_on; then
    state=$(gh issue view "$n" --json state -q .state)
    [ "$state" = OPEN ] || die "#$n is $state"
  fi

  # A held claim is exit 2 even when its worktree is here, so the session
  # that lost knows to pick another issue.
  wt=$(worktree_of "$n")
  if [ -z "$steal" ]; then
    read -r old old_t <<<"$(remote_claim "$n")"
    [ -z "${old:-}" ] || held "$n" "$old" "$old_t" "$wt"
  fi

  # Checked before claiming, so a refusal leaves nothing to undo. A worktree
  # or branch already here is another session's, or a stale one: never adopt it.
  if [ -n "$make_wt" ]; then
    [ ! -e "$wt" ] \
      || die "$wt already exists: another session's worktree, or a stale one. Leave it alone unless it's stale ('issue.sh list', 'git -C $wt status'); then push or drop its work, remove it, and claim again"
    ! git show-ref -q --verify "refs/heads/issue-$n" \
      || die "branch issue-$n already exists here. If it's stale, push or drop its work, delete it, and claim again"
  fi

  agent=${CLAIM_AGENT_ID:-$(hostname)-$(now)-$RANDOM}
  sha=$(make_claim "$n" "$agent")

  if [ -n "$steal" ]; then
    read -r old old_t <<<"$(remote_claim "$n")"
    [ -n "${old:-}" ] || die "#$n isn't claimed; claim it without --steal"
    [ $(( $(now) - old_t )) -ge $(( TTL_HOURS * 3600 )) ] \
      || die "#$n claim by $(agent_of "$old") is $(age_text "$old_t") old, under the ${TTL_HOURS}h TTL"
    # Compare-and-swap: fails if the holder renewed since we looked.
    git push -q --force-with-lease="$ref:$old" "$REMOTE" "$sha:$ref" \
      || die "#$n claim changed while stealing it; run list and look again"
  elif ! git push -q "$REMOTE" "$sha:$ref" 2>/dev/null; then
    read -r old old_t <<<"$(remote_claim "$n")"
    [ -n "${old:-}" ] || die "push of the claim for #$n failed, and nobody holds it; check the network and retry"
    held "$n" "$old" "$old_t" "$wt"
  fi

  mkdir -p "$(record_dir)"
  if [ -n "$make_wt" ]; then
    # Continue an earlier agent's pushed work in progress, if there is any.
    git fetch -q "$REMOTE" master
    base=$REMOTE/master
    if git fetch -q "$REMOTE" "refs/heads/issue-$n:refs/remotes/$REMOTE/issue-$n" 2>/dev/null; then
      base=$REMOTE/issue-$n
    fi
    if ! git worktree add -q "$wt" -b "issue-$n" "$base"; then
      git push -q --force-with-lease="$ref:$sha" "$REMOTE" ":$ref" || true
      die "couldn't make the worktree $wt, so the claim on #$n was dropped"
    fi
    wt=$(git -C "$wt" rev-parse --show-toplevel)
    printf '%s\n%s\n' "$agent" "$wt" >"$(record_of "$n")"
  else
    echo "$agent" >"$(record_of "$n")"
  fi
  if gh_on; then
    gh issue edit "$n" --add-label "$LABEL" >/dev/null || true
    gh issue comment "$n" --body "Claimed by \`$agent\` on \`$(hostname)\`${steal:+ (taken over from a stale claim)}." >/dev/null || true
  fi
  echo "claimed #$n as $agent"
  [ -z "$make_wt" ] || echo "worktree: $wt (branch issue-$n from $base). Work there; renew, release and land run only from it"
}

cmd_renew() {
  local n=${1:?issue number} old new
  old=$(require_mine "$n")
  new=$(make_claim "$n" "$(record_agent "$n")" "$old")
  git push -q "$REMOTE" "$new:$(ref_of "$n")" || die "renew of #$n was rejected"
  echo "renewed #$n"
}

cmd_release() {
  local n=${1:?issue number} sha
  sha=$(require_mine "$n")
  git push -q --force-with-lease="$(ref_of "$n"):$sha" "$REMOTE" ":$(ref_of "$n")" \
    || die "release of #$n was rejected"
  rm -f "$(record_of "$n")"
  gh_on && { gh issue edit "$n" --remove-label "$LABEL" >/dev/null 2>&1 || true; }
  echo "released #$n"
}

cmd_land() {
  local n=${1:?issue number} verify="" attempt
  shift
  [ "${1:-}" = --verify ] && verify=${2:?--verify needs a command}
  require_mine "$n" >/dev/null
  git diff --quiet && git diff --cached --quiet || die "commit or drop your changes first"
  git fetch -q "$REMOTE" master
  git log "$REMOTE/master..HEAD" --format=%B | grep -qi "closes #$n\b" \
    || echo "issue.sh: warning: no commit says 'Closes #$n'" >&2

  # The commit --verify last passed on. Verify runs before the first push and
  # again whenever catching up with master changes HEAD, so a land after a
  # conflict resolved by hand is still verified.
  local verified=""
  for attempt in 1 2 3 4 5; do
    if ! git merge-base --is-ancestor "$REMOTE/master" HEAD; then
      # A rebase flattens merges, and a subtree pull's merge is what the next
      # `git subtree pull`/`push` finds its base by. So a branch holding any
      # merge, or a squashed subtree commit, merges master in instead.
      if [ -n "$(git rev-list --merges "$REMOTE/master..HEAD")" ] \
        || git log "$REMOTE/master..HEAD" --format=%B | grep -q '^git-subtree-dir:'; then
        echo "master moved; merging it in to keep the branch's merges (attempt $attempt)"
        git merge -q --no-edit "$REMOTE/master" || {
          echo "issue.sh: merge conflict. Resolve it, 'git commit', then land again." >&2
          exit 3
        }
      else
        echo "master moved; rebasing (attempt $attempt)"
        git rebase -q "$REMOTE/master" || {
          echo "issue.sh: rebase conflict. Resolve it, 'git rebase --continue', then land again." >&2
          exit 3
        }
      fi
    fi
    if [ -n "$verify" ] && [ "$verified" != "$(git rev-parse HEAD)" ]; then
      echo "verifying: $verify"
      bash -c "$verify" || die "verify failed; fix it, commit, land again"
      verified=$(git rev-parse HEAD)
    fi
    # Fast-forward only: a rejection means someone landed first, so loop.
    if git push -q "$REMOTE" HEAD:master; then
      echo "landed #$n at $(git rev-parse --short HEAD)"
      cmd_release "$n"
      git push -q "$REMOTE" --delete "issue-$n" 2>/dev/null || true
      return 0
    fi
    git fetch -q "$REMOTE" master
  done
  die "master kept moving; land again"
}

case ${1:-} in
  list) cmd_list ;;
  claim) shift; cmd_claim "$@" ;;
  renew) shift; cmd_renew "$@" ;;
  release) shift; cmd_release "$@" ;;
  land) shift; cmd_land "$@" ;;
  *) usage ;;
esac
