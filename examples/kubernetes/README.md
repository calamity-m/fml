# Kubernetes JSON Firehose Demo

A minimal Helm chart for reproducing high-rate Kubernetes log ingestion with long randomized JSON lines.

The chart deploys uneven producers:

- `json-firehose-hot`: high-volume pods emitting sustained long JSON logs
- `json-firehose-slow`: low-volume pods emitting the same JSON shape much less frequently

That uneven mix is intentional. It models the production case where a few deployments flood the log stream while you filter or focus on a quieter deployment.

Each pod emits logs with:

- lines typically longer than 200 characters
- randomized trace IDs, request IDs, users, tenants, paths, statuses, levels, regions, and payloads
- configurable burst size and sleep interval
- occasional very large JSON lines from the hot producers
- mostly stdout with some error logs on stderr

It intentionally avoids real app dependencies, external Helm charts, databases, and image builds. The goal is to stress `fml --producer kubernetes` with the log shape and uneven source rates that exposed bugs in real work usage.

See `docs/local-k8s-testing.md` for k3d cluster setup before running this.

## Install

```sh
helm install fml-demo examples/kubernetes
```

Run fml against it:

```sh
cargo run -p fml -- --producer kubernetes
```

In fml, try filtering/focusing sources from `json-firehose-slow` while the `json-firehose-hot` pods continue flooding logs.

## Tune load

Default values start 4 hot pods and 2 slow pods.

Increase hot pod count:

```sh
kubectl scale deployment json-firehose-hot --replicas=10
```

Make the quiet deployment even quieter:

```sh
helm upgrade fml-demo examples/kubernetes \
  --set firehoses[1].linesPerBatch=1 \
  --set firehoses[1].batchSleepMs=5000
```

Install with more aggressive hot output:

```sh
helm install fml-demo examples/kubernetes \
  --set firehoses[0].replicas=8 \
  --set firehoses[0].linesPerBatch=2500 \
  --set firehoses[0].batchSleepMs=25 \
  --set firehoses[0].minPayloadChars=300 \
  --set firehoses[0].maxPayloadChars=3000
```

Max out local stress by removing the hot batch sleep:

```sh
helm upgrade fml-demo examples/kubernetes \
  --set firehoses[0].replicas=10 \
  --set firehoses[0].linesPerBatch=5000 \
  --set firehoses[0].batchSleepMs=0 \
  --set firehoses[0].minPayloadChars=300 \
  --set firehoses[0].maxPayloadChars=4000
```

## Simulate churn

Scale down and back up to trigger source removal and discovery:

```sh
kubectl scale deployment json-firehose-hot --replicas=0
kubectl scale deployment json-firehose-hot --replicas=4
```

Rolling restart reconnects streams one by one:

```sh
kubectl rollout restart deployment/json-firehose-slow
```

## Uninstall

```sh
helm uninstall fml-demo
```
