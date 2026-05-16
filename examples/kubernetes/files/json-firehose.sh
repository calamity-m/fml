#!/bin/sh
set -eu

LINES_PER_BATCH=${LINES_PER_BATCH:-1000}
BATCH_SLEEP_MS=${BATCH_SLEEP_MS:-100}
MIN_PAYLOAD_CHARS=${MIN_PAYLOAD_CHARS:-220}
MAX_PAYLOAD_CHARS=${MAX_PAYLOAD_CHARS:-1800}
HUGE_EVERY=${HUGE_EVERY:-5000}
SERVICE_NAME=${SERVICE_NAME:-json-firehose}
POD_NAME=${HOSTNAME:-json-firehose}

awk \
  -v lines_per_batch="$LINES_PER_BATCH" \
  -v batch_sleep_ms="$BATCH_SLEEP_MS" \
  -v min_payload_chars="$MIN_PAYLOAD_CHARS" \
  -v max_payload_chars="$MAX_PAYLOAD_CHARS" \
  -v huge_every="$HUGE_EVERY" \
  -v service_name="$SERVICE_NAME" \
  -v pod_name="$POD_NAME" '
BEGIN {
  srand(systime() + length(pod_name));
  levels[0] = "debug"; levels[1] = "info"; levels[2] = "info"; levels[3] = "info"; levels[4] = "warn"; levels[5] = "error";
  methods[0] = "GET"; methods[1] = "POST"; methods[2] = "PUT"; methods[3] = "PATCH"; methods[4] = "DELETE";
  paths[0] = "/api/orders"; paths[1] = "/api/orders/search"; paths[2] = "/api/checkout"; paths[3] = "/api/customers";
  paths[4] = "/api/inventory/reserve"; paths[5] = "/api/payments/authorize"; paths[6] = "/api/recommendations"; paths[7] = "/internal/events";
  regions[0] = "us-east-1"; regions[1] = "us-west-2"; regions[2] = "eu-west-1"; regions[3] = "ap-southeast-2";
  messages[0] = "request completed"; messages[1] = "downstream call completed"; messages[2] = "cache lookup completed";
  messages[3] = "validation failed"; messages[4] = "retrying downstream dependency"; messages[5] = "database query completed";
  alphabet = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
  count = 0;

  while (1) {
    timestamp = strftime("%Y-%m-%dT%H:%M:%SZ", systime(), 1);
    trace_prefix = hex(8) hex(8);
    burst = lines_per_batch + int(rand() * (lines_per_batch / 2 + 1));

    for (i = 0; i < burst; i++) {
      count++;
      level = levels[int(rand() * 6)];
      method = methods[int(rand() * 5)];
      path = paths[int(rand() * 8)];
      region = regions[int(rand() * 4)];
      message = messages[int(rand() * 6)];
      status = choose_status(level);
      duration = int(rand() * 2400) + 1;
      user_id = int(rand() * 90000000) + 10000000;
      tenant_id = int(rand() * 9000) + 1000;
      payload_len = min_payload_chars + int(rand() * (max_payload_chars - min_payload_chars + 1));
      if (huge_every > 0 && count % huge_every == 0) {
        payload_len = max_payload_chars * 8;
        level = "warn";
        message = "large json payload sampled";
      }
      payload = random_string(payload_len);
      trace_id = trace_prefix hex(8) sprintf("%08x", count % 4294967295);
      span_id = hex(8) hex(8);
      request_id = sprintf("req-%s-%06d-%06d", pod_name, count, int(rand() * 1000000));

      line = sprintf("{\"ts\":\"%s\",\"level\":\"%s\",\"service\":\"%s\",\"pod\":\"%s\",\"region\":\"%s\",\"message\":\"%s\",\"trace_id\":\"%s\",\"span_id\":\"%s\",\"request_id\":\"%s\",\"tenant_id\":%d,\"user_id\":%d,\"http\":{\"method\":\"%s\",\"path\":\"%s\",\"status\":%d,\"duration_ms\":%d,\"bytes_in\":%d,\"bytes_out\":%d},\"kubernetes\":{\"namespace\":\"default\",\"pod\":\"%s\",\"container\":\"json-firehose\"},\"labels\":{\"app\":\"firehose\",\"version\":\"v%d.%d.%d\",\"shard\":\"%02d\"},\"payload\":{\"cart_id\":\"cart-%08d\",\"feature_flags\":[\"new_checkout\",\"recommendations\",\"risk_scoring\"],\"random\":\"%s\"}}",
        timestamp, level, service_name, pod_name, region, message, trace_id, span_id, request_id, tenant_id, user_id,
        method, path, status, duration, int(rand() * 20000), int(rand() * 250000), pod_name,
        int(rand() * 4) + 1, int(rand() * 20), int(rand() * 30), int(rand() * 32), int(rand() * 100000000), payload);

      if (level == "error" && rand() < 0.30) {
        print line > "/dev/stderr";
      } else {
        print line;
      }
    }
    fflush("");
    if (batch_sleep_ms > 0) {
      system("sleep " (batch_sleep_ms / 1000));
    }
  }
}

function choose_status(level) {
  if (level == "error") return 500 + int(rand() * 4);
  if (level == "warn") return (rand() < 0.5) ? 429 : 409;
  if (rand() < 0.04) return 404;
  return 200 + int(rand() * 4);
}

function random_string(len, out, i, idx) {
  out = "";
  for (i = 0; i < len; i++) {
    idx = int(rand() * length(alphabet)) + 1;
    out = out substr(alphabet, idx, 1);
  }
  return out;
}

function hex(len, out, i) {
  out = "";
  for (i = 0; i < len; i++) {
    out = out sprintf("%x", int(rand() * 16));
  }
  return out;
}
'
