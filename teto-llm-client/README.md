# teto-llm-client

Parent-side client for the private `teto-llm-worker`. The crate does not link
llama.cpp and contains no inference code.

`Client::open(WorkerLaunch, OpenConfig, ClientOptions)` starts the worker and
opens one model. The typed methods are `start`, `pull`, `acknowledge`, `cancel`,
`finish`, `reset`, `retry` and `close`.

`finish` takes a `ReceiptSource`. A display client uses `Display`. A speech
client uses `Playback` and acknowledges each clause after device playback
completes.

`WorkerLaunch` is explicit. `WorkerLaunch::from_environment` is a convenience
for the CLI. `JournalAdmin` handles initialize, inspect and prepared delete.

The crate has no audio and no global tracing subscriber. See
[the contract](../llm-subsystem.md).
