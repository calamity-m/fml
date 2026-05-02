i=0
while true; do
  level=info
  event=job_complete
  if [ $((i % 9)) -eq 0 ]; then level=warn; event=retry_scheduled; fi
  if [ $((i % 23)) -eq 0 ]; then level=error; event=job_failed; fi
  printf 'level=%s service=worker event=%s queue=emails job_id=job-%04d attempt=%s duration_ms=%s\n' "$level" "$event" "$i" $((1 + i % 4)) $((120 + i % 700))
  i=$((i + 1))
  sleep 0.9
done
