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
#   flint    in flint-client/: capitol-core's and capitol-client's tests
#
# The db step needs DATABASE_URL. A TheCapitol-<n> worktree has no
# backend/.env, so when it's unset this borrows DATABASE_URL and REDIS_URL
# from the main checkout's.
#
# The db step holds a lock on the Postgres it tests against (#227). sqlx names
# each test's database after the test's path, so two checkouts running the
# same test on one server share a database, and each drops it under the other
# (55006, "being accessed by other users"). A database per checkout wouldn't
# help: database names are global to the server. So the tests build first,
# then wait their turn. The lock is a directory in the temp dir named after
# the server's host and port, so every checkout on the machine shares it; a
# lock whose holder has died is taken over.
#
# Use it as land's verify: scripts/issue.sh land <n> --verify "scripts/ci.sh"

set -euo pipefail

die() { echo "ci.sh: $*" >&2; exit 1; }

usage() { sed -n '7,16p' "$0" | sed 's/^# \{0,1\}//'; exit 1; }

ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"

# Ignored tests that are tools, not tests: one prints migration rows, one
# renders suburb plans, one draws the wilds (water_plan), one reports the wild
# nodes (wild_node_report), one writes the Gazette's fixture
# (write_the_gazette_fixture), and two write the default world to a path from an
# env var ($TERRAIN_DUMP, $TERRAIN_FIXTURE). A new ignored tool goes here.
DB_SKIP=(print_node_moves suburb_plan_report water_plan wild_node_report write_the_gazette_fixture dump_default_terrain capture_terrain_fixture)

# KEY's value in the main checkout's backend/.env, if it has one.
main_env() {
    local main env
    main=$(dirname "$(cd "$(git rev-parse --git-common-dir)" && pwd)")
    env="$main/backend/.env"
    [ -f "$env" ] || return 0
    sed -n "s/^$1=//p" "$env" | tr -d '\r' | tail -1
}

# The lock for the Postgres at URL, as a directory under the temp dir.
DB_LOCK=
db_lock() {
    local hostport dir owner pid waited=0
    hostport=$(printf '%s' "$1" | sed -E 's#^[a-z]+://([^@/]*@)?([^/?]*).*#\2#; s#[^A-Za-z0-9]#_#g')
    dir="${TMPDIR:-/tmp}/capitol-ci-db-${hostport:-default}.lock"
    until mkdir "$dir" 2>/dev/null; do
        owner=$(cat "$dir/owner" 2>/dev/null || true)
        pid=${owner%% *}
        if [ -n "$pid" ] && ! kill -0 "$pid" 2>/dev/null; then
            echo "== ci.sh: db: taking the lock from $owner, which has gone" >&2
            rm -rf "$dir"
            continue
        fi
        if [ -z "$owner" ] && [ -n "$(find "$dir" -maxdepth 0 -mmin +1 2>/dev/null)" ]; then
            rm -rf "$dir"   # made but never written: its holder died at once
            continue
        fi
        if [ $((waited % 60)) -eq 0 ]; then
            echo "== ci.sh: db: waiting for ${owner:-another run} to finish with the database (${waited}s)" >&2
        fi
        sleep 5
        waited=$((waited + 5))
    done
    echo "$$ $(basename "$ROOT") $(date '+%H:%M:%S')" > "$dir/owner"
    DB_LOCK=$dir
    trap 'rm -rf "$DB_LOCK"' EXIT
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
            cargo test -p capitol-backend --no-run
            db_lock "$DATABASE_URL"
            cargo test -p capitol-backend -- --ignored "${skips[@]}"
            rm -rf "$DB_LOCK"; DB_LOCK=; trap - EXIT ;;
        flint)
            # Per crate, never --workspace: the engine's flint-android needs
            # ndk-sys. capitol-core's live.rs tests need a server and stay
            # #[ignore]d. The client's tests need no GPU or window; until
            # #234 it was only checked, and #225's items went without icons.
            (cd flint-client && cargo test -p capitol-core && cargo test -p capitol-client --all-targets) ;;
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

# The Rust is rust-toolchain.toml's, the same as CI's (#265). RUSTUP_TOOLCHAIN
# beats the file, so say when it's set: a verify on another version can pass
# what CI fails, or the other way round.
if [ -n "${RUSTUP_TOOLCHAIN:-}" ]; then
    echo "== ci.sh: warning: RUSTUP_TOOLCHAIN=$RUSTUP_TOOLCHAIN overrides rust-toolchain.toml; CI won't match" >&2
fi
echo "== ci.sh: $(rustc --version)" >&2

if [ $# -eq 0 ]; then
    set -- clippy test db
    if touches_flint; then set -- "$@" flint; fi
fi
for s in "$@"; do step "$s"; done
echo "== ci.sh: passed ($*)" >&2
