#!/usr/bin/env bash
# Live check: run Nook against a real server and print what it did.
#
#   scripts/live-check.sh postgres://localhost/shop
#   scripts/live-check.sh mysql://root@127.0.0.1/shop
#
# Read-only except for the scratch database the sync and structure checks use, which it creates and
# drops. Safe to run against a development server; do not point it at production.
set -euo pipefail

CONN="${1:-}"
if [ -z "$CONN" ]; then
  echo "usage: scripts/live-check.sh <connection uri>" >&2
  exit 2
fi

cd "$(dirname "$0")/.."
cargo build -q -p nook-cli
CLI=./target/debug/nook-cli

echo "== connection =="
$CLI sql "$CONN" "select 1 as ok" || exit 1

echo "== catalog =="
$CLI tables "$CONN" | head -20

echo "== server =="
$CLI monitor "$CONN" --status | head -20

echo "== users =="
$CLI users "$CONN" | head -10

echo "== grid edits on a scratch table =="
SCRATCH="nook_live_check_$$"
$CLI sql "$CONN" "create table $SCRATCH (id integer primary key, note text)" >/dev/null
$CLI sql "$CONN" "insert into $SCRATCH values (1, 'one'), (2, 'two')" >/dev/null
$CLI sql "$CONN" "update $SCRATCH set note = 'edited' where id = 1" | tail -1
$CLI sql "$CONN" "select * from $SCRATCH order by id" | head -6
$CLI describe "$CONN" "$SCRATCH" | head -12

echo "== backup and restore =="
OUT="$(mktemp -d)/check-$$.sql"
$CLI dump "$CONN" --out "$OUT" --tables "$SCRATCH" || true
if [ -s "$OUT" ]; then
  echo "backup: $(wc -c < "$OUT") bytes"
  $CLI restore "$CONN" --file "$OUT" | head -3 || true
fi

echo "== diagram =="
SVG="$(mktemp -d)/check-$$.svg"
$CLI diagram "$CONN" --out "$SVG" || true
[ -s "$SVG" ] && echo "svg: $(wc -c < "$SVG") bytes"

echo "== chart =="
$CLI chart "$CONN" "select note as label, count(*) as n from $SCRATCH group by note" --chart column | head -8

echo "== clean up =="
$CLI sql "$CONN" "drop table $SCRATCH" >/dev/null
echo "done"
