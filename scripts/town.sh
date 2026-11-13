#!/usr/bin/env bash
# A town: a second backend with citizens living in it, beside the usual one
# (#337). The usual world is `cargo run` from backend/, on the database
# `capitol` and port 3000, and nothing puts citizens in it. The town has a
# database of its own, a port of its own and a Redis db of its own, so its
# citizens never claim a lot, take a vein or fill a book in the world you
# work in, and either can run without the other, or both at once.
#
#   scripts/town.sh up [--online n] [--merchants m] [--always-on]
#                           make what's missing, then run the town's backend
#                           and its citizens in this terminal; Ctrl-C stops both
#   scripts/town.sh status  the town's server, and its citizens' last health
#   scripts/town.sh reset   drop the town's database and roster, for a new town
#
# The first `up` creates the database, an admin for it and a roster of
# citizens. Citizens keep human hours (#276): each signs in when its habit
# says, a few real hours some days, in its own evenings, so the roster is
# sized for how many are on at once: enough of the sim's mix to keep
# --online on (18) and enough merchants, trading favourites, to keep
# --merchants on (2), on average. Since Callings K2.2 anyone leaning to
# trading makes a market once it has its float, so --merchants 0 still
# trades. That's about 32 citizens for each one on, so about 640 in all.
# `capitol-town habits --roster flint-client/town/town.toml` shows who's on
# when. --always-on keeps every citizen on, whatever its habit: all of
# them, so mind the pool's size. Later ups reuse the database, the admin and
# the roster, so the town carries on where it stopped. Then play in it with
#
#   flint-client\play.bat --server http://127.0.0.1:3300
#
# and in the usual world with play.bat as ever. A new town knows nobody, so
# register an account on the client's form the first time. The client keeps
# a sign-in for each server, so switching back and forth needs none (#338).
#
# What the town keeps is in flint-client/town/ (git-ignored: the roster holds
# the citizens' keys, and admin.env the admin's password), with the backend's
# log. It runs copies of the backend and capitol-town (target/town/ and
# flint-client/target/town/), so a running town never locks the exe that
# `cargo run` rebuilds, nor the other way round.
#
# Environment:
#   TOWN_PORT   the town's port (default 3300)
#   TOWN_DB     its database, in the docker-compose Postgres (default capitol_town)
#   TOWN_REDIS  its Redis db index (default 1; the usual world's is 0)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
port="${TOWN_PORT:-3300}"
db="${TOWN_DB:-capitol_town}"
redis_db="${TOWN_REDIS:-1}"
server="http://127.0.0.1:$port"
dir="$root/flint-client/town"
roster="$dir/town.toml"
admin_env="$dir/admin.env"
postgres=capitol-postgres

die() { echo "town: $*" >&2; exit 1; }
say() { echo "town: $*"; }

psql_q() { # psql_q <database> <sql>: one value, no decoration
  docker exec "$postgres" psql -U capitol -d "$1" -tAc "$2"
}

healthy() { curl -fsS -m 2 "$server/health" >/dev/null 2>&1; }

exe() { # the platform's name for a binary
  case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) echo "$1.exe" ;; *) echo "$1" ;; esac
}

need_postgres() {
  docker exec "$postgres" pg_isready -U capitol >/dev/null 2>&1 \
    || die "Postgres ($postgres) isn't up: run docker-compose up -d at the repo root"
}

ensure_database() {
  if [[ "$(psql_q capitol "select 1 from pg_database where datname = '$db'")" != 1 ]]; then
    say "creating the database $db"
    psql_q capitol "create database $db" >/dev/null
  fi
}

# Builds a binary and copies it where only the town runs it. If the build
# fails because the usual one is running and holds the exe, the last copy
# will do, with a warning.
build_copy() { # build_copy <workspace dir> <package> <bin> <copy dir>
  local ws="$1" package="$2" bin="$3" to="$4"
  local built="$ws/target/debug/$(exe "$bin")" copy="$to/$(exe "$bin")"
  say "building $bin"
  if (cd "$ws" && cargo build -q -p "$package" --bin "$bin"); then
    mkdir -p "$to"
    cp "$built" "$copy" 2>/dev/null \
      || say "can't replace the copy of $bin (is a town running it?); running the last copy"
  elif [[ -x "$copy" ]]; then
    say "the build of $bin failed (is the usual one running and holding its exe?); running the last copy"
  else
    die "the build of $bin failed, and there's no earlier copy to run"
  fi
}

backend_pid=""
town_pid=""

stop() {
  trap - INT TERM
  echo
  say "stopping: the citizens finish their steps (up to 20 s), then the server"
  # On Windows the console's Ctrl-C reached both already. Elsewhere a
  # background job ignores it, so pass it on.
  if [[ "$(exe x)" == x ]]; then
    for pid in $town_pid $backend_pid; do kill -INT "$pid" 2>/dev/null || true; done
  fi
  # Wait for the town, then make sure of the server.
  if [[ -n "$town_pid" ]]; then
    for _ in $(seq 25); do kill -0 "$town_pid" 2>/dev/null || break; sleep 1; done
    kill "$town_pid" 2>/dev/null || true
  fi
  if [[ -n "$backend_pid" ]]; then
    for _ in $(seq 10); do kill -0 "$backend_pid" 2>/dev/null || break; sleep 1; done
    kill "$backend_pid" 2>/dev/null || true
  fi
  say "stopped"
  exit 0
}

# The server reads backend/.env (its JWT_SECRET) from its working directory.
# A worktree has none: it's git-ignored. Then start_backend hands the main
# checkout's to the server, as ci.sh's main_env does (#580), and nothing is
# copied. Dies when neither has one.
main_env=
find_backend_env() {
  [[ -f "$root/backend/.env" ]] && return 0
  local main
  main=$(dirname "$(cd "$(git -C "$root" rev-parse --path-format=absolute --git-common-dir)" && pwd)")
  [[ -f "$main/backend/.env" ]] || die "backend/.env is missing here and in the main checkout ($main): copy backend/.env.example there (the town reads its JWT_SECRET)"
  main_env="$main/backend/.env"
  say "this worktree has no backend/.env: the server reads the main checkout's ($main_env)"
}

start_backend() {
  build_copy "$root" capitol-backend capitol-backend "$root/target/town"
  mkdir -p "$dir"
  local log="$dir/backend.log"
  say "starting the town's server on $server (its log: flint-client/town/backend.log)"
  # The variables set here win over backend/.env's: dotenvy fills only what's unset.
  # A worktree's server takes the main checkout's (#580), under these.
  (cd "$root/backend" && \
    if [[ -n "$main_env" ]]; then
      while IFS='=' read -r name value; do
        export "$name=${value%$'\r'}"
      done < <(grep -E '^[A-Za-z_][A-Za-z0-9_]*=' "$main_env")
    fi && \
    DATABASE_URL="postgres://capitol:capitol_dev@localhost:5433/$db" \
    REDIS_URL="redis://localhost:6379/$redis_db" \
    HOST=127.0.0.1 PORT="$port" \
    exec "$root/target/town/$(exe capitol-backend)") >"$log" 2>&1 &
  backend_pid=$!
  local waited=0
  until healthy; do
    if ! kill -0 "$backend_pid" 2>/dev/null; then
      tail -n 20 "$log" >&2
      die "the town's server stopped before it was up (above, the end of its log)"
    fi
    # A new database runs every migration and plants every node first.
    if (( waited > 0 && waited % 30 == 0 )); then say "still starting ($waited s)"; fi
    sleep 1
    waited=$((waited + 1))
  done
  say "the server is up"
}

# The town's admin mints its citizens. It's made once, on the town's own
# database, with a password nobody types.
ensure_admin() {
  if [[ ! -f "$admin_env" ]]; then
    local email="clerk@town.capitol.local"
    local password
    password="$(od -An -N18 -tx1 /dev/urandom | tr -d ' \n')"
    say "making the town's admin, $email"
    curl -fsS -m 10 -H 'Content-Type: application/json' \
      -d "{\"email\":\"$email\",\"password\":\"$password\",\"player_name\":\"TownClerk\"}" \
      "$server/auth/register" >/dev/null \
      || die "couldn't register the town's admin (does $email exist on $db already? try reset)"
    printf 'CAPITOL_ADMIN_EMAIL=%s\nCAPITOL_ADMIN_PASSWORD=%s\n' "$email" "$password" >"$admin_env"
  fi
  set -a
  # shellcheck source=/dev/null
  . "$admin_env"
  set +a
  psql_q "$db" "update accounts set is_admin = true where email = '$CAPITOL_ADMIN_EMAIL' and not is_admin" >/dev/null
}

ensure_citizens() { # ensure_citizens <online> <merchants online>
  if [[ -f "$roster" ]] && grep -q '^\[\[citizen\]\]' "$roster"; then
    say "the roster has $(grep -c '^\[\[citizen\]\]' "$roster") citizens"
    return
  fi
  local town_bin="$root/flint-client/target/town/$(exe capitol-town)"
  say "minting enough of the sim's mix to keep $1 on, and enough trading favourites to keep $2 on"
  (cd "$root/flint-client" && "$town_bin" mint --online "$1" --roster "$roster" --server "$server" | tail -n 1)
  if [[ "$2" != 0 ]]; then
    (cd "$root/flint-client" && "$town_bin" mint --online "$2" --role merchant --roster "$roster" --server "$server" | tail -n 1)
  fi
}

up() {
  local online=18 merchants=2 hours=()
  while (( $# )); do
    case "$1" in
      --online) online="$2"; shift 2 ;;
      --merchants) merchants="$2"; shift 2 ;;
      --always-on) hours=(--always-on); shift ;;
      *) die "up takes --online <n>, --merchants <m> and --always-on, not $1" ;;
    esac
  done
  find_backend_env
  if healthy; then
    die "something already answers on $server: another town? Stop it, or set TOWN_PORT"
  fi
  need_postgres
  ensure_database
  build_copy "$root/flint-client" capitol-core capitol-town "$root/flint-client/target/town"
  trap stop INT TERM
  start_backend
  ensure_admin
  ensure_citizens "$online" "$merchants"
  say "the town lives. Play in it: flint-client\\play.bat --server $server"
  say "Ctrl-C stops the citizens and the server"
  (cd "$root/flint-client" && exec "$root/flint-client/target/town/$(exe capitol-town)" run --roster "$roster" --server "$server" "${hours[@]}") &
  town_pid=$!
  # Wait on the town; if it ends by itself (every key refused, say), stop.
  wait "$town_pid" || true
  town_pid=""
  stop
}

status() {
  if healthy; then say "the server answers on $server"; else say "nothing answers on $server"; fi
  if [[ -f "$dir/town.status" ]]; then
    "$root/flint-client/target/town/$(exe capitol-town)" status --roster "$roster" \
      || cat "$dir/town.status"
  else
    say "no citizens have run here yet"
  fi
}

reset() {
  if healthy; then die "the town is running on $server: stop it first"; fi
  need_postgres
  say "dropping the database $db and the roster in flint-client/town/"
  psql_q capitol "drop database if exists $db with (force)" >/dev/null
  # Only the town's own files: the roster, its keys, its health and logs.
  rm -rf "$dir"
  redis_flush
  say "done: the next up makes a new town"
}

# The town's sessions and refresh tokens name accounts that are gone now.
redis_flush() {
  docker exec capitol-redis redis-cli -n "$redis_db" flushdb >/dev/null 2>&1 || true
}

case "${1:-}" in
  up) shift; up "$@" ;;
  status) status ;;
  reset) reset ;;
  *) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 2 ;;
esac
