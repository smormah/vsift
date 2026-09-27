#!/usr/bin/env bash
# Checks the P10 crash campaign's acceptance numbers (ADR 0020 section 7)
# across the evidence every layer uploaded, and prints them as Markdown.
#
# A: >= 2,000 replay points, no lost acknowledgement, no damage, clean fsck;
#    the negative control lost at least one acknowledgement.
# B: >= 300 kills in total, every cycle recovered, fsck-clean and verified.
# C: at least one injected failure, every one STORAGE_IO, none acknowledged,
#    the final verification clean.
#
# Usage: acceptance.sh <evidence dir>
set -uo pipefail

evidence=$1
failures=0
field() { sed -n "s/.* $1=\([0-9]*\).*/\1/p" <<< " $2" | head -n 1; }
fail() { echo "- **FAIL**: $*"; failures=$(( failures + 1 )); }

echo "## P10 durability campaign"
echo

positive=$(cat "$evidence/layer-a-positive/result.txt" 2>/dev/null || true)
negative=$(cat "$evidence/layer-a-negative/result.txt" 2>/dev/null || true)
echo "- Layer A: \`${positive:-missing}\`"
echo "- Negative control: \`${negative:-missing}\`"
points=$(field points "$positive")
if [ "$(field status "$positive")" != 0 ] || [ "${points:-0}" -lt 2000 ] \
  || [ "$(field lost_acks "$positive")" != 0 ] || [ "$(field damaged_points "$positive")" != 0 ] \
  || [ "$(field fsck_failures "$positive")" != 0 ] || [ "$(field mount_failures "$positive")" != 0 ]; then
  fail "layer A needs >= 2000 clean replay points with no lost acknowledgement"
fi
if [ "${negative_lost:=$(field lost_acks "$negative")}" = "" ] || [ "$negative_lost" -lt 1 ]; then
  fail "the negative control lost no acknowledgement, so the harness cannot see loss"
fi

kills=0
cycle_failures=0
shards=0
for result in "$evidence"/layer-b-shard-*/result.txt; do
  [ -f "$result" ] || continue
  line=$(cat "$result")
  echo "- Layer B $(basename "$(dirname "$result")"): \`$line\`"
  kills=$(( kills + $(field kills "$line") ))
  cycle_failures=$(( cycle_failures + $(field cycle_failures "$line") ))
  shards=$(( shards + 1 ))
done
echo "- Layer B total: $kills kills over $shards shards, $cycle_failures failed cycles"
if [ "$kills" -lt 300 ] || [ "$cycle_failures" != 0 ] || [ "$shards" != 4 ]; then
  fail "layer B needs >= 300 kills over four shards with every cycle verified"
fi

layer_c=$(cat "$evidence/layer-c/result.txt" 2>/dev/null || true)
echo "- Layer C: \`${layer_c:-missing}\`"
injected=$(field injected_failures "$layer_c")
typed=$(field storage_io "$layer_c")
if [ "${injected:-0}" -lt 1 ] || [ "$injected" != "$typed" ] || [ "$(field final_status "$layer_c")" != 0 ] \
  || [ "$(field fsck_nonzero "$layer_c")" != 0 ] || [ "$(field rounds "$layer_c")" != "$(field passed "$layer_c")" ]; then
  fail "layer C needs every injected failure typed STORAGE_IO and never acknowledged"
fi

echo
if [ "$failures" = 0 ]; then
  echo "**All acceptance criteria met.**"
else
  echo "**$failures acceptance criteria not met.**"
  exit 1
fi
