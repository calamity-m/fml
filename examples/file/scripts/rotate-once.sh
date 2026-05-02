#!/bin/sh
set -eu

path="${1:-examples/file/logs/api.log}"
dir="$(dirname "$path")"
base="$(basename "$path")"
stamp="$(date +%Y%m%d%H%M%S)"

mkdir -p "$dir"
if [ -e "$path" ]; then
  mv "$path" "$dir/$base.$stamp"
fi

: > "$path"
printf '{"level":"warn","msg":"rotated log file","path":"%s","rotated_to":"%s"}\n' "$path" "$dir/$base.$stamp" >> "$path"
echo "rotated $path"
