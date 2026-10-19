#!/usr/bin/env bash
# The CI steps, by name, so an agent runs before `land` exactly what
# .github/workflows/ci.yml runs after it (#219). The workflow calls this
# script, so the two can't drift. Before it, every issue verified with
# `cargo test` alone and clippy failed on master for 31 pushes (#216).
#
#   scripts/ci.sh                 clippy, test and db, plus flint when the
#                                 branch touches flint-client/ or protocol/
#   scripts/ci.sh <step>...       just these steps, in this order
#
# Steps:
#   clippy   clippy over the backend and protocol, all targets, -D warnings
#   test     cargo test, backend and protocol
#   db       the #[ignore]d database tests; needs Postgres (docker-compose up -d)
#   flint    in flint-client/: capitol-core's tests, check capitol-client
#
# The db step needs DATABASE_URL. A TheCapitol-<n> worktree has no
# backend/.env, so when it's unset this borrows DATABASE_URL and REDIS_URL
# from the main checkout's.
#
# Use it as land's verify: scripts/issue.sh land <n> --verify "scripts/ci.sh"

set -euo pipefail

die() { echo "ci.sh: $*" >&2; exit 1; }

usage() { sed -n '7,16p' "$0" | sed 's/^# \{0,1\}//'; exit 1; }

ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"

# Ignored tests that are tools, not tests: one prints migration rows, one
# renders suburb plans, one draws the wilds (water_plan), one reports the wild
# nodes (wild_node_report), and two write the default world to a path from an
# env var ($TERRAIN_DUMP, $TERRAIN_FIXTURE). A new ignored tool goes here.
DB_SKIP=(print_node_moves suburb_plan_report water_plan wild_node_report dump_default_terrain capture_terrain_fixture)

# KEY's value in the main checkout's backend/.env, if it has one.
main_env() {
    local main env
    main=$(dirname "$(cd "$(git rev-parse --git-common-dir)" && pwd)")
    env="$main/backend/.env"
    [ -f "$env" ] || return 0
    sed -n "s/^$1=//p" "$env" | tr -d '\r' | tail -1
}

step() {
    echo "== ci.sh: $1" >&2
    case $1 in
        clippy)
            cargo clippy --all-targets -p capitol-protocol -p capitol-backend -- -D warnings ;;
        test)
            cargo test -p capitol-protocol -p capitol-backend ;;
        db)
            if [ -z "${DATABASE_URL:-}" ]; then
                DATABASE_URL=$(main_env DATABASE_URL)
                [ -n "$DATABASE_URL" ] || die "db: set DATABASE_URL, or give the main checkout a backend/.env"
                export DATABASE_URL
                : "${REDIS_URL:=$(main_env REDIS_URL)}"
                [ -z "$REDIS_URL" ] || export REDIS_URL
            fi
            local skips=()
            for t in "${DB_SKIP[@]}"; do skips+=(--skip "$t"); done
            cargo test -p capitol-backend -- --ignored "${skips[@]}" ;;
        flint)
            # Per crate, never --workspace: the engine's flint-android needs
            # ndk-sys. capitol-core's live.rs tests need a server and stay
            # #[ignore]d.
            (cd flint-client && cargo test -p capitol-core && cargo check -p capitol-client --all-targets) ;;
        *)
            usage ;;
    esac
}

# Does this branch change anything the Flint client builds from?
touches_flint() {
    local base
    base=$(git merge-base HEAD origin/master 2>/dev/null) || return 0
    git diff --name-only "$base" HEAD | grep -qE '^(flint-client|protocol)/'
}

if [ $# -eq 0 ]; then
    set -- clippy test db
    if touches_flint; then set -- "$@" flint; fi
fi
for s in "$@"; do step "$s"; done
echo "== ci.sh: passed ($*)" >&2
