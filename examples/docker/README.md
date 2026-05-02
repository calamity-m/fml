# Docker Producer Demo

This compose stack starts several small containers that continuously emit
different log shapes so `fml --producer docker` has useful data to show:

- JSON application logs
- logfmt-style worker logs
- web access logs
- stdout and stderr interleaving
- a restart-looping short-lived task to demonstrate source churn

Start the demo logs:

```sh
docker compose -f examples/docker/docker-compose.yml up
```

In another terminal, run fml:

```sh
cargo run -p fml -- --producer docker
```

The Docker producer groups these containers under the compose project name
`fml-demo`. The services use shell scripts in `examples/docker/scripts`, mounted
read-only into each container.

Stop the demo with:

```sh
docker compose -f examples/docker/docker-compose.yml down
```
