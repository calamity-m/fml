i=0
while true; do
  method=GET
  path=/health
  status=200
  if [ $((i % 5)) -eq 0 ]; then path=/search; method=POST; fi
  if [ $((i % 13)) -eq 0 ]; then path=/checkout; status=500; fi
  printf '10.0.0.%s - - [02/May/2026:12:%02d:%02d +0000] "%s %s HTTP/1.1" %s %s "-" "fml-demo/%s"\n' $((1 + i % 8)) $((i % 60)) $((i * 3 % 60)) "$method" "$path" "$status" $((300 + i * 17 % 4000)) "$i"
  i=$((i + 1))
  sleep 0.7
done
