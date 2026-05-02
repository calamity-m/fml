i=0
while true; do
  level=info
  status=200
  if [ $((i % 11)) -eq 0 ]; then level=warn; status=429; fi
  if [ $((i % 17)) -eq 0 ]; then level=error; status=503; fi
  printf '{"ts":"2026-05-02T12:%02d:%02dZ","level":"%s","msg":"handled request","route":"/api/orders","status":%s,"request_id":"req-%04d","latency_ms":%s}\n' $((i % 60)) $((i * 7 % 60)) "$level" "$status" "$i" $((35 + i % 190))
  i=$((i + 1))
  sleep 0.6
done
