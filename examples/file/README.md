# File Producer Demo

This example writes several live log files under `examples/file/logs` so the
file producer can be exercised without Docker or Kubernetes:

- JSON application logs
- logfmt-style worker logs
- web access logs
- plain text stderr-style logs
- optional rename/recreate rotation

Start the log generator:

```sh
sh examples/file/scripts/generate.sh
```

In another terminal, run fml against one or more generated files:

```sh
cargo run -p fml -- \
  --producer file:examples/file/logs/api.log \
  --producer file:examples/file/logs/worker.log \
  --producer file:examples/file/logs/access.log \
  --producer file:examples/file/logs/noisy.log
```

The file producer starts tailing from EOF, so start fml while the generator is
still running. To demonstrate rotation handling, run:

```sh
sh examples/file/scripts/rotate-once.sh examples/file/logs/api.log
```

Stop the generator with `ctrl+c`. Remove generated logs with:

```sh
rm -rf examples/file/logs
```
