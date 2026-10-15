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
#   scripts/issue.sh claim <n>                claim issue n (exit 2 if held)
#   scripts/issue.sh claim <n> --steal        take over a claim older than the TTL
#   scripts/issue.sh renew <n>                refresh your claim's timestamp
#   scripts/issue.sh release <n>              give up your claim
#   scripts/issue.sh land <n> [--verify CMD]  rebase on master, push to master,
#                                             release the claim
#
# Environment:
#   CLAIM_REMOTE     remote to use (default origin)
#   CLAIM_TTL_HOURS  age after which a claim may be stolen (default 24)
#   CLAIM_AGENT_ID   name to claim under (default <host>-<time>-<random>)
#   CLAIM_NO_GH=1    skip GitHub calls (labels, comments, issue state); for tests

set -euo pipefail

REMOTE=${CLAIM_REMOTE:-origin}
TTL_HOURS=${CLAIM_TTL_HOURS:-24}
LABEL=claimed


die() { echo "issue.sh: $*" >&2; exit 1; }

usage() { sed -n '12,19p' "$0" | sed 's/^# \{0,1\}//'; exit 1; }

gh_on() { [ "${CLAIM_NO_GH:-}" != 1 ]; }

ref_of() { echo "refs/heads/claim/issue-$1"; }

# Local record of the claims this checkout made, shared by all its worktrees.
# Only the machine that claimed an issue can renew, release or land it.
record_dir() { echo "$(git rev-parse --path-format=absolute --git-common-dir)/agent-claims"; }
record_of() { echo "$(record_dir)/issue-$1"; }

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

# Fails unless this checkout holds the claim. Prints the remote claim sha.
require_mine() {
  local n=$1 rec sha _t
  rec=$(record_of "$n")
  [ -f "$rec" ] || die "#$n wasn't claimed from this checkout (no $rec)"
  read -r sha _t <<<"$(remote_claim "$n")"
  [ -n "${sha:-}" ] || die "#$n has no claim on $REMOTE"
  [ "$(agent_of "$sha")" = "$(cat "$rec")" ] \
    || die "#$n is now held by $(agent_of "$sha"), not $(cat "$rec")"
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

cmd_claim() {
  local n=${1:?issue number} steal=${2:-} ref agent sha old old_t state
  ref=$(ref_of "$n")
  if gh_on; then
    state=$(gh issue view "$n" --json state -q .state)
    [ "$state" = OPEN ] || die "#$n is $state"
  fi
  agent=${CLAIM_AGENT_ID:-$(hostname)-$(now)-$RANDOM}
  sha=$(make_claim "$n" "$agent")

  if [ "$steal" = --steal ]; then
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
    echo "#$n is already claimed by $(agent_of "$old"), $(age_text "$old_t") ago" >&2
    exit 2
  fi

  mkdir -p "$(record_dir)"
  echo "$agent" >"$(record_of "$n")"
  if gh_on; then
    gh issue edit "$n" --add-label "$LABEL" >/dev/null || true
    gh issue comment "$n" --body "Claimed by \`$agent\` on \`$(hostname)\`${steal:+ (taken over from a stale claim)}." >/dev/null || true
  fi
  echo "claimed #$n as $agent"
}

cmd_renew() {
  local n=${1:?issue number} old new
  old=$(require_mine "$n")
  new=$(make_claim "$n" "$(cat "$(record_of "$n")")" "$old")
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

  for attempt in 1 2 3 4 5; do
    if ! git merge-base --is-ancestor "$REMOTE/master" HEAD; then
      echo "master moved; rebasing (attempt $attempt)"
      git rebase -q "$REMOTE/master" || {
        echo "issue.sh: rebase conflict. Resolve it, 'git rebase --continue', re-verify, then land again." >&2
        exit 3
      }
      if [ -n "$verify" ]; then
        echo "re-verifying: $verify"
        bash -c "$verify" || die "verify failed after rebase; fix it, commit, land again"
      fi
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
