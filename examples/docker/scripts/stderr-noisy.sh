i=0
while true; do
  printf 'INFO cache refresh key=feature-flags generation=%s\n' "$i"
  if [ $((i % 4)) -eq 0 ]; then printf 'WARN cache stale key=session-%s age_ms=%s\n' "$i" $((9000 + i)) >&2; fi
  if [ $((i % 10)) -eq 0 ]; then printf 'ERROR upstream timeout peer=payments timeout_ms=2500 trace=trace-%04d\n' "$i" >&2; fi
  i=$((i + 1))
  sleep 0.8
done
