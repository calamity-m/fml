#!/bin/sh
set -eu

root="$(dirname "$0")/.."
log_dir="$root/logs"
mkdir -p "$log_dir"

pids=""
cleanup() {
  for pid in $pids; do
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup INT TERM EXIT

write_api() {
  i=0
  while true; do
    level=info
    status=200
    if [ $((i % 10)) -eq 0 ]; then level=warn; status=429; fi
    if [ $((i % 19)) -eq 0 ]; then level=error; status=503; fi
    printf '{"ts":"2026-05-02T12:%02d:%02dZ","level":"%s","msg":"file api request","route":"/v1/items","status":%s,"request_id":"file-req-%04d","latency_ms":%s}\n' $((i % 60)) $((i * 5 % 60)) "$level" "$status" "$i" $((25 + i % 240)) >> "$log_dir/api.log"
    i=$((i + 1))
    sleep 0.5
  done
}

write_worker() {
  i=0
  while true; do
    level=info
    event=job_done
    if [ $((i % 8)) -eq 0 ]; then level=warn; event=retry; fi
    if [ $((i % 21)) -eq 0 ]; then level=error; event=dead_letter; fi
    printf 'level=%s service=file-worker event=%s queue=imports job_id=file-job-%04d attempt=%s duration_ms=%s\n' "$level" "$event" "$i" $((1 + i % 3)) $((90 + i % 850)) >> "$log_dir/worker.log"
    i=$((i + 1))
    sleep 0.8
  done
}

write_access() {
  i=0
  while true; do
    method=GET
    path=/ready
    status=200
    if [ $((i % 6)) -eq 0 ]; then method=POST; path=/upload; fi
    if [ $((i % 14)) -eq 0 ]; then path=/checkout; status=500; fi
    printf '127.0.0.%s - - [02/May/2026:12:%02d:%02d +0000] "%s %s HTTP/1.1" %s %s "-" "fml-file-demo/%s"\n' $((1 + i % 5)) $((i % 60)) $((i * 4 % 60)) "$method" "$path" "$status" $((200 + i * 13 % 5000)) "$i" >> "$log_dir/access.log"
    i=$((i + 1))
    sleep 0.7
  done
}

write_noisy() {
  i=0
  while true; do
    printf 'INFO disk scan completed path=/tmp/fml-demo generation=%s\n' "$i" >> "$log_dir/noisy.log"
    if [ $((i % 5)) -eq 0 ]; then printf 'WARN slow filesystem path=/tmp/fml-demo elapsed_ms=%s\n' $((1200 + i)) >> "$log_dir/noisy.log"; fi
    if [ $((i % 12)) -eq 0 ]; then printf 'ERROR failed to stat path=/tmp/fml-demo/missing-%s errno=ENOENT\n' "$i" >> "$log_dir/noisy.log"; fi
    i=$((i + 1))
    sleep 0.9
  done
}

write_api & pids="$pids $!"
write_worker & pids="$pids $!"
write_access & pids="$pids $!"
write_noisy & pids="$pids $!"

echo "writing logs under $log_dir"
wait
