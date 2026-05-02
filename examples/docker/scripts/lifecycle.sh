i=0
while [ "$i" -lt 8 ]; do
  printf '{"level":"info","msg":"short lived task heartbeat","task":"rollup","step":%s}\n' "$i"
  i=$((i + 1))
  sleep 0.5
done
printf '{"level":"error","msg":"short lived task exiting to demonstrate container churn","task":"rollup"}\n'
exit 1
