# msfs-async

A native Windows, runtime-independent asynchronous SimConnect client built on
the raw bindings from `msfs-rs`.

The public client can be cloned and used from ordinary async tasks. A dedicated
driver thread owns the SimConnect handle, waits on a Windows event supplied to
`SimConnect_Open`, and routes responses to typed futures and streams.

```rust
let sim = AsyncSimConnect::open("MY CLIENT").await?;

let current = sim
    .request_once::<AircraftData>(SIMCONNECT_OBJECT_ID_USER)
    .await?;

let updates = sim
    .subscribe::<AircraftData>(SIMCONNECT_OBJECT_ID_USER, RecurringPeriod::SimFrame)
    .await?;

sim.set_data_on_sim_object(SIMCONNECT_OBJECT_ID_USER, &updated).await?;
```

For an AI object whose position or attitude the client will drive, release the
AI controller and freeze the components that the simulator would otherwise
overwrite:

```rust
sim.release_ai_control(target_id).await?;
sim.set_freeze(target_id, FreezeState::ALL).await?;
```

Both calls are serialized on the SimConnect driver thread. Their futures report
whether the local SDK call was accepted; delayed server-side failures are sent
to `sim.exceptions()`.

`request_once` is a future which resolves to one owned value. `subscribe`
returns a `Stream<Item = Result<T>>`; dropping that stream sends the same
request with the native disabled period and unregisters its route.

For frame telemetry, latest-value delivery avoids processing stale buffered
poses when a consumer falls behind:

```rust
let updates = sim
    .subscribe_with_options::<AircraftData>(
        SIMCONNECT_OBJECT_ID_USER,
        SubscriptionOptions::new(RecurringPeriod::SimFrame).latest(),
    )
    .await?;
```

`SubscriptionOptions` also exposes the native changed-only, origin, interval,
and limit settings. `event_stream()` combines heterogeneous typed
subscriptions; its mapping functions run in the polling task, not on the
native driver thread.

The `#[data_definition]` and `#[client_data_definition]` macros generate the C
layout, `Copy` implementation, SDK definition trait, and internal safety marker.
They reject fields which cannot safely be populated from SimConnect bytes. In
particular, use `i32` rather than Rust `bool` for simulation variables, and
`u8` or `i32` rather than `bool` for externally writable client data.

## Examples

- [`request_once.rs`](examples/request_once.rs) awaits several typed one-shot
  requests concurrently.
- [`log.rs`](examples/log.rs) joins several stream consumers in the current
  task.
- [`task_per_stream.rs`](examples/task_per_stream.rs) spawns an independent
  consumer task for each data type.
- [`select_streams.rs`](examples/select_streams.rs) multiplexes several typed
  streams in one `tokio::select!` loop.
- [`client_data.rs`](examples/client_data.rs) ports the upstream client-data
  writer/reader example onto two asynchronous SimConnect sessions.
- [`set_data.rs`](examples/set_data.rs) writes a simulation-object data
  definition, listens for delayed server exceptions, and reads the value back.
- [`read_write_same_client.rs`](examples/read_write_same_client.rs) writes from
  one task and consumes a typed subscription through the same SimConnect client.
- [`relay_simobject.rs`](examples/relay_simobject.rs) copies live, SimFrame-paced
  position and attitude from one SimObject ID to another.
- [`replay_simobject.rs`](examples/replay_simobject.rs) reads timestamped poses
  from CSV and resamples them at a fixed output rate, using wrapped longitude
  interpolation and quaternion slerp for attitude.

The replay CSV has these columns:

```text
timestamp_seconds,latitude_degrees,longitude_degrees,altitude_feet,pitch_degrees,bank_degrees,heading_degrees
```

Its optional arguments default to 60 Hz and real-time playback:

```console
cargo run -p msfs-async --example replay_simobject -- flight.csv 42 60 1
```

Run one on a Windows machine with the MSFS SDK installed:

```console
cargo run -p msfs-async --example request_once
```

Tokio is only used by the examples; the library itself is runtime-independent.

The upstream NVG example is not ported: it is a WASM cockpit gauge using the
NanoVG and gauge lifecycle APIs, while this crate currently targets native
Windows SimConnect clients only.
