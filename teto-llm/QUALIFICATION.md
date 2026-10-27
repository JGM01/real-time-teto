# LLM qualification evidence

Status: Policy 2. The desktop warm soak, the 20-process cold start, the Pro Max cache probe and the Pro Max cache run are complete. The lifecycle soak and audio are pending.

## Identity

Desktop: Mac15,7, Apple M3 Pro, 36 GiB, macOS 27.0.1. Native runtime: `llama-cpp-2` and `llama-cpp-sys-2` exactly 0.1.154, llama.cpp `5f55650a78f92aff4d48d671423e888fac0469ff`, Metal, full SWA cache. Rust 1.88.0. The model and template hashes are in [the policy registry](../teto-llm-protocol/src/policy.rs).

The worker verifies the model file with SHA-256. A metadata-keyed cache avoids a full hash when the file size, modification time and file identity are unchanged. The cache path is explicit. A missing or changed cache entry causes a full hash.

## Desktop warm soak

The run uses the release CLI, the private worker, Whisper resident in the parent and an encrypted journal. It runs for 1,804 seconds. [Turn records](qualification/desktop-policy2-run.log) and [identities](qualification/desktop-policy2-resources.json) are preserved.

| Measurement | Observed |
|---|---:|
| Turns | 2,885 |
| First-clause p50 / p95, all turns | 347 ms / 450 ms |
| First-clause p50 / p95, warm sample | 340 ms / 441 ms |
| Cache-hit turns | 2,806 |
| Cache-hit first-clause p95 | 444 ms |
| Context rebuilds | 79 (2.7 percent of turns) |
| Evicted exchanges | 2,807 |
| Generation throughput | 26.17 tokens/s |
| Generation failures | 0 |
| Interrupted playback | 0 |

The one-second p95 target is met on the desktop path. The worst observed clause in the run is a rebuild turn at 4.63 seconds. A rebuild evaluates 2,474 prompt tokens in 3.67 seconds. The p95 excludes that tail because fewer than five percent of turns rebuild.

## Cold start

Twenty independent children run with ASR resident and an encrypted journal. The identity cache and the operating-system page cache are warm. [Cold run](qualification/desktop-cold-policy2.log).

| Measurement | Observed |
|---|---:|
| Cold process p50 / p95 | 1.54 s / 1.56 s |
| First clause after model load | 348 ms |
| Identity cache | warm |

A two-turn real-model run takes 32.9 seconds with a cold identity cache and 2.2 seconds with a warm identity cache. The difference is the full-file SHA-256. A first launch includes the hash.

## Cache capability

The desktop probe is recorded in [cache-m3-pro.json](qualification/cache-m3-pro.json). The phone probe is recorded in [cache-BA02436A-2530-4F66-A435-06D2C18F0B47.jsonl](qualification/phone-pro-max/cache-BA02436A-2530-4F66-A435-06D2C18F0B47.jsonl). Both cover append, normalized history, token limit, interruption, eviction and forced removal failure. All six phone cases return equal greedy continuations. Every non-forced case removes the suffix.

Policy 2 enables `VerifiedPrefixReuse` for the desktop artifact on Mac15,7/macOS 27.0.1 and the phone artifact on iPhone16,2/iOS 27.0.1. Other hosts use `FullRebuild`. The phone probe matched the exact model and template hashes in the registry.

## Phone qualification

The run uses the release `teto-llm` crate directly on an iPhone 16,2 (iPhone 15 Pro Max), iOS 27.0.1. The iPhone 15 Pro Max is the qualification device. [Report](qualification/phone-pro-max/spike-60FB536F-1EB4-4FAC-88A5-172D3FE1F5B6.jsonl).

| Measurement | Observed |
|---|---:|
| Turns | 269 |
| Duration | 301 s |
| First-clause p50 / p95, all turns | 674 ms / 816 ms |
| First-clause p50 / p95, warm sample | 438 ms / 764 ms |
| Cache mode | VerifiedPrefixReuse |
| Cache reasons | 263 reused, 5 eviction, 1 cold |
| Context rebuilds | 6 |
| Evicted exchanges | 180 |
| Generation throughput | 15.46 tokens/s |
| Cancellation probe | 2.59 ms |
| Generation failures | 0 |
| Longest turn | 14.25 s (rebuild) |

The one-second first-clause target is met on the phone. The tail contains six rebuild turns; the p95 excludes that tail. Resource sampling produced three samples in this run, so memory and thermal values are not reported for this run.

## Delivery-window experiment

The mailbox supports one or two delivered clauses. The default is one. The two-clause window allows one clause to play while the next is prepared. Unit tests cover prefetch, contiguous receipts and the complete-seal fence.

The audio gap measurement is pending the audio sink. The experiment does not claim a latency improvement yet.

## Probes and smoke

A 2,230-token prompt evaluates in 3.62 seconds at 616 tokens per second with the 512-token batch. The first clause on that full rebuild is 4.35 seconds.

A 120-turn engine run measures first-clause p50 332 milliseconds and p95 450 milliseconds. The run records two context rebuilds and 34 evicted exchanges in 66.4 seconds. The run uses the engine directly and excludes ASR, IPC and journal writes.

All nine real-model tests pass on the desktop host. The suite covers replacement, reset, close, drop, context reuse, a cancelled prefix, the token limit with an encrypted journal, encrypted finalization, and the cancellation probe. It runs in 28.6 seconds with a warm identity cache.

The five-case CLI smoke run passes with the real worker: text follow-up and EOF, Ctrl-C during output, broken stdout with failed playback finalization, an active child crash, and encrypted journal deletion.

## Targets

| Measurement | Target | Status |
|---|---:|---|
| Desktop first-clause p95 | 1,000 ms | met at 450 ms |
| Context rebuilds per 100 turns | at most 5 | met at 2.7 |
| Desktop generation throughput | 20 tokens/s | met at 26.17 |
| Cold start p95, 20 processes | 20 s | met at 1.56 s with a warm identity cache |
| Asset hash after the cache is warm | 100 ms | met; the warm path skips the hash |
| Cancellation p95 | 500 ms | probe only; the soak does not interrupt |
| Cache probe | six cases on the production prompt | met on desktop and Pro Max |
| Phone first-clause p95 | 1,000 ms | met at 816 ms |
| Phone generation throughput | 20 tokens/s | missed at 15.46 |

## Pending runs

1. The lifecycle soak with interrupts, reset and clean process reopen.
2. Real TTS, first audible sample, starvation and complete audio latency.

## Method

First-clause latency is parent wall time from `Start` to the first clause. All times are integer microseconds. Percentiles use nearest rank. Generation throughput excludes prompt evaluation and queue wait. [Summary](qualification/summary.json) is generated by `python3 tools/summarize_qualification.py`.
