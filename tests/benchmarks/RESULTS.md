# Shared-Instance Benchmark Results

These results were collected on March 31, 2026 with the benchmark harness in this directory.

At the time of these runs, the harness used `senders_per_user=1`. The harness now supports `--senders-per-user > 1` so future runs can exercise per-tenant concurrency limits directly.

All runs used:

- one local IronClaw instance
- mock OpenAI-compatible LLM backend
- libSQL
- `20` users
- `5` measured messages per user
- `20` max in-flight requests
- no tools

## Run Matrix

| Label | SSE/user | Runtime | Extra knobs | Requests | p50 first | p95 first | p99 first | p50 final | p95 final | p99 final | SSE delivery | CPU avg / max | RSS avg / max |
|------|---------:|---------|-------------|---------:|----------:|----------:|----------:|----------:|----------:|----------:|-------------:|--------------:|--------------:|
| `baseline-20u-2sse` | 2 | `multi_thread` | none | 100 | 18.97 ms | 1785.61 ms | 2220.03 ms | 35.17 ms | 1807.29 ms | 2240.16 ms | 1.000 / 1.000 | 68.01 / 96.30 % | 89.50 / 92.05 MB |
| `fanout-20u-4sse` | 4 | `multi_thread` | none | 100 | 18.56 ms | 1534.93 ms | 1935.07 ms | 34.68 ms | 1554.27 ms | 1954.12 ms | 1.000 / 1.000 | 85.15 / 98.90 % | 98.94 / 102.00 MB |
| `runtime-current-thread-20u-2sse` | 2 | `current_thread` | none | 100 | 18.90 ms | 1491.55 ms | 1881.39 ms | 35.23 ms | 1509.86 ms | 1899.91 ms | 1.000 / 1.000 | 83.03 / 98.00 % | 87.05 / 90.12 MB |
| `lower-agent-jobs-8-20u-2sse` | 2 | `multi_thread` | `AGENT_MAX_PARALLEL_JOBS=8` | 100 | 15.78 ms | 1479.68 ms | 1874.28 ms | 31.67 ms | 1500.67 ms | 1893.37 ms | 1.000 / 1.000 | 82.02 / 97.80 % | 86.43 / 90.09 MB |
| `tenant-llm-1-20u-2sse` | 2 | `multi_thread` | `TENANT_MAX_LLM_CONCURRENT=1` | 100 | 22.63 ms | 1470.68 ms | 1859.03 ms | 38.77 ms | 1489.23 ms | 1878.14 ms | 1.000 / 1.000 | 79.52 / 97.80 % | 87.54 / 90.12 MB |

`SSE delivery` is reported as `any-event delivery / final-event delivery`.

## Observed Bottlenecks

### 1. Tail latency is the first visible bottleneck

Across every `20`-user run, median latency stayed low while p95 and p99 rose sharply:

- p50 first-event latency stayed around `16-23 ms`
- p95 first-event latency stayed around `1.47-1.79 s`
- p99 first-event latency stayed around `1.86-2.22 s`

That pattern points to burst queueing or scheduler contention under concurrent load rather than a uniformly slow request path.

### 2. CPU pressure rises before SSE fanout breaks

Increasing from `2` to `4` SSE streams per user:

- kept delivery perfect at `1.000 / 1.000`
- increased CPU average from `68.01%` to `85.15%`
- increased RSS average from `89.50 MB` to `98.94 MB`

So the first fanout cost in this workload is higher process overhead, not dropped SSE delivery.

### 3. No SSE loss was observed up to 80 open streams

The `fanout-20u-4sse` run held `80` SSE connections open and all expected streams received both any-event and final-event delivery for every measured request.

At this workload level, SSE correctness and per-user fanout looked stable.

### 4. Current-thread runtime did not regress this workload

The `current_thread` run completed cleanly and slightly improved long-tail latency relative to the baseline run on this machine:

- p95 first-event: `1491.55 ms` vs `1785.61 ms`
- p99 first-event: `1881.39 ms` vs `2220.03 ms`

This is only one local sample, so it should be treated as directional rather than conclusive, but it does support keeping the benchmark-only runtime gate.

## Important Workload Limitations

### Per-user concurrency knobs are not strongly exercised yet

The current harness uses:

- one sender task per user
- sequential message submission within each user

That means `TENANT_MAX_LLM_CONCURRENT` is only weakly exercised, and `TENANT_MAX_JOBS_CONCURRENT` is not meaningfully exercised at all because this workload does not launch jobs.

The `tenant-llm-1-20u-2sse` result stayed close to the other runs, which is consistent with the workload shape rather than proof that the knob is irrelevant.

If we want to benchmark per-user concurrency limits directly, the harness needs a `senders_per_user` or per-user in-flight request knob.

## Takeaways

- The first meaningful performance signal is high tail latency under bursty shared-instance chat load.
- SSE fanout looked robust through `80` open streams, but it raises CPU and memory usage.
- `current_thread` is worth keeping in the benchmark matrix because it did not obviously lose on this workload.
- The next benchmark improvement should be adding parallel senders per user so per-tenant concurrency limits can be tested directly.

## Harness Update: Multi-Sender Validation

After the runs above, the harness was extended with `--senders-per-user` so requests can overlap within the same user.

Two small validation runs completed successfully:

| Label | Users | SSE/user | Senders/user | Messages/user | Extra knobs | Requests | p50 first | p95 first | p50 final | p95 final |
|------|------:|---------:|-------------:|--------------:|-------------|---------:|----------:|----------:|----------:|----------:|
| `multisender-smoke` | 6 | 2 | 2 | 4 | none | 24 | 58.46 ms | 693.35 ms | 78.72 ms | 709.40 ms |
| `multisender-tenant-llm-1-smoke` | 6 | 2 | 2 | 4 | `TENANT_MAX_LLM_CONCURRENT=1` | 24 | 51.00 ms | 671.41 ms | 69.53 ms | 688.55 ms |

These were only correctness-oriented validation runs, not full comparison runs, but they confirm that the harness can now overlap requests inside the same tenant and is ready for a fuller per-user-concurrency benchmark pass.
