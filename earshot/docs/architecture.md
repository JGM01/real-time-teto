# Architecture

This page describes internal behavior. Rustdoc is the API reference. The README describes use.

## Threads and ownership

`Earshot::open` starts a coordinator thread. The coordinator starts two engine workers before it reports readiness:
- The VAD worker owns the Silero engine.
- The endpoint worker owns the Smart Turn ONNX session.

Each engine worker lives until the owner closes or an engine fails.

`Earshot` owns the event receiver. `Control` owns a command sender and a share of the coordinator state. `Control` is cloneable. The coordinator owns activation state.

Each activation that starts capture creates one capture producer. The producer owns the device stream or the replay clock. The VAD worker joins the producer before it reports the end of the capture.

## Coordinator state machine

The coordinator has four states:
- `Idle`: no activation is active.
- `Capturing`: one activation is active.
- `Finalizing`: capture has stopped and the coordinator waits for the VAD worker to finish.
- `OutcomePending`: the terminal event is queued and the coordinator waits for the owner to receive it.

The main transitions are:
1. `Idle` receives `Activate`. The coordinator creates an ID, creates the endpoint, sets `Capturing`, and sends a capture request to the VAD worker.
2. `Capturing` receives `Ready`. The coordinator emits `Listening` and starts the capture clock.
3. `Capturing` receives `Frame`. The endpoint observes the frame and may stop the capture.
4. `Capturing` receives a model result. The coordinator stores the result as pending evidence.
5. `Capturing` reaches the decision target with current positive evidence. The coordinator starts finalization as `Complete`.
6. `Capturing` observes cancellation. The coordinator starts finalization as `Cancelled`.
7. `Capturing` or `Finalizing` receives the VAD end message. The coordinator builds the terminal outcome and emits `Finished`.
8. `OutcomePending` sees that the owner received the outcome. It returns to `Idle`.

A worker exit sets shutdown. The coordinator emits one `Failed` outcome for an active attempt, if one exists, and stops.

The event channel has capacity for one `Listening` and one `Finished` event. Admission is released by receipt, not by downstream work.

## Clocks and decision target
After capture is ready, the coordinator uses one of two clock modes:
- Realtime: the clock reads the capture watermark on the normalized timeline. It also schedules a no-speech deadline.
- Simulated: the clock advances by samples and pauses while semantic inference is in flight. This mode is for accelerated replay.

A positive model result is not used immediately. The coordinator stores the speech revision and a decision target. The result becomes usable only when the capture reaches the target. Resumed speech increments the revision and makes old evidence stale.

This rule prevents a model result from completing a statement on audio that the coordinator has not yet observed. It applies to both clock modes.

## Capture pipeline

The capture path has five stages:
1. Source: the default microphone or a replay source.
2. Producer: the device callback or the replay clock pushes native frames into a bounded ring.
3. Normalizer: the consumer mixes stereo to mono, converts the sample format, and resamples to 16 kHz.
4. Frame accumulator: the VAD worker copies normalized samples into 512-sample frames and checks that sample positions are contiguous.
5. VAD: the Silero engine scores each complete frame.

The ring capacity is 2048 ms at the native rate. The callback uses no locks, allocations, or logging. The producer counts complete frames and dropped frames.

If the ring is full, the producer drops the newest frames and counts them. The consumer turns a nonzero drop count into `CaptureOverrun`. The pipeline does not drop audio silently.

The normalizer uses the same path for microphone input and replay input. A native rate of 16 kHz copies samples without resampling. Other rates use a persistent FFT resampler and discard its initial output delay.

## Endpoint policy

The endpoint owns the retained waveform. The model never changes the waveform.

The endpoint has two phases:
- Waiting: it retains up to the last ten VAD frames. Ten frames cover 256 ms of pre-speech audio plus the two onset frames. Onset requires two consecutive frames with a probability of at least 0.5.
- Speech: it appends frames to the retained audio. It updates the last speech position on frames with a probability of at least 0.35. It opens a pause on a frame below 0.35.

A pause creates at most one model snapshot. The snapshot ends at the pause boundary and contains up to 8 s of audio. The coordinator reuses one snapshot buffer.

Stop conditions:
- No speech before the 5 s no-speech limit.
- Retained audio reaches the 300 s duration limit.
- Five seconds pass after the last speech frame.

Delivery:
- `Complete` returns the speech tail plus up to a 256 ms trailing window.
- `Incomplete(SilenceLimit)` returns the speech tail plus up to the trailing window.
- `Incomplete(DurationLimit)` returns all retained audio.
- `NoSpeech`, `Cancelled`, and `Failed` return no audio.

The endpoint returns its audio arena to the coordinator. The next activation reuses the arena.

## Worker failure and panic handling

Each named worker catches panics. A panic becomes `WorkerPanicked` with the worker name. A worker that stops without a report becomes `WorkerExited`.

The coordinator treats a worker exit as a service failure. It sets shutdown, stops the current capture stream, and publishes `Failed` for the active attempt. `Control::activate` then returns `Closed`.

## Bounded diagnostic traces

The `tooling` feature enables diagnostic types. The default build uses zero-storage types and has no trace code.

With `tooling`, a trace can contain:
- Up to 308 s of captured samples.
- Up to 2048 model evaluations.
- Timing values and the SHA-256 of the bundled model.

The audit harness uses a one-slot trace channel. A slow consumer receives omitted-trace counts, and the next trace is marked incomplete. Traces can contain raw audio, so treat them as sensitive data. They are diagnostic evidence, not public API.
