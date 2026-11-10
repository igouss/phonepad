#!/bin/sh
dir=$(dirname "$0")
echo "$1" >> "$dir/calls"
case "$1" in
  status) if [ -e "$dir/paused" ]; then echo '{"manual_pause":true}'; else echo '{"manual_pause":false}'; fi;;
  pause) touch "$dir/paused";;
  resume) rm -f "$dir/paused";;
esac
