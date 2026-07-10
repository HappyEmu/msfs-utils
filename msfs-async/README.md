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

Create a non-ATC AI aircraft, then release the AI controller and freeze the
components that the client will drive:

```rust
let aircraft = sim
    .create_non_atc_aircraft(model_title, tail_number, initial_position)
    .await?;
let target_id = aircraft.object_id();

sim.release_ai_control(target_id).await?;
sim.set_freeze(target_id, FreezeState::ALL).await?;
// Repeatedly call set_data_on_sim_object while driving the aircraft.
sim.remove_object(target_id).await?;
```

Creation resolves only after SimConnect assigns an object ID. If its future is
cancelled during the assignment handoff, the driver removes the otherwise
orphaned aircraft. Dropping a successfully returned `AiAircraft` does not remove
it; lifecycle owners should call `remove_object` explicitly. The control and
removal calls report whether the local SDK call was accepted, while delayed
server-side failures are sent to `sim.exceptions()`.

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
particular, use an explicitly requested integer representation such as `i32` or
`i64` rather than Rust `bool` for simulation variables, and `u8` or `i32`
rather than `bool` for externally writable client data.

## Wire-layout verification

Microsoft's published documentation and the distributed `SimConnect.h` disagree
about `SIMCONNECT_RECV_SIMOBJECT_DATA::dwDefineCount`. The web documentation
describes it as the number of 8-byte elements in `dwData`, while the header calls
it the number of datums and explicitly says it is not a byte count. The driver
therefore does not derive payload length from `dwDefineCount`: the packet's
`dwSize` bounds all reads, and the registered Rust type validates the exact
number of payload bytes it decodes.

The portable driver suite exercises both interpretations against the same mixed
16-byte payload. To determine what a particular SDK/runtime actually emits,
start MSFS on Windows and run:

```console
cargo run -p msfs-async --example inspect_wire_layout
```

The probe requests `INT32`, `FLOAT64`, and `INT32` in that order. It prints
the simulator and SimConnect versions, `dwSize`, `dwDefineCount`, payload length,
raw bytes, packed-offset values, and C-aligned-offset values. Preserve that
output when checking a new SDK version.

A live Windows run on 2026-07-10 with simulator `SunRise 12.2` build
`282174.999` and SimConnect `12.2` build `0.0` returned `dwSize = 56`,
`dwDefineCount = 3`, and a 16-byte payload. The three values decoded correctly
at packed offsets 0, 4, and 12. This matches the distributed header: the count
is the number of datums, and scalar payload fields are packed without C alignment
padding. The driver remains independent of the disputed count semantics because
it uses `dwSize` and the registered Rust type size for bounds validation.

## Live simulator tests

The ignored `live_simconnect` integration suite exercises behavior which a fake
backend cannot validate: opening the installed SDK, requesting mixed live data,
writing and reading an L-variable, native subscription limits under a full local
buffer, padded client-data exchange between two sessions, shared-handle shutdown,
and optional AI-aircraft creation/removal.

Start MSFS and fully load a flight, then run the suite serially:

```console
cargo test -p msfs-async --test live_simconnect -- --ignored --test-threads=1 --nocapture
```

Normal `cargo test` runs skip these tests. The AI lifecycle test defaults to
`Airbus A320 Neo Asobo`. Override it with another exact installed container
title when necessary:

```powershell
$env:MSFS_TEST_AIRCRAFT_TITLE = "Airbus A320 Neo Asobo"
```

Use a dedicated test flight: the suite writes `L:MSFS_ASYNC_LIVE_TEST_VALUE`,
creates a named client-data area, and creates and removes an AI aircraft.

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
- [`spawn_aircraft.rs`](examples/spawn_aircraft.rs) creates an AI aircraft near
  the user, drives its pose for five seconds, and removes it explicitly.
- [`inspect_wire_layout.rs`](examples/inspect_wire_layout.rs) reports the raw
  mixed-datatype receive layout and `dwDefineCount` semantics of a running SDK.

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

The lifecycle example needs an installed aircraft container title:

```console
cargo run -p msfs-async --example spawn_aircraft -- "Airbus A320 Neo Asobo"
```

Tokio is only used by the examples; the library itself is runtime-independent.

The upstream NVG example is not ported: it is a WASM cockpit gauge using the
NanoVG and gauge lifecycle APIs, while this crate currently targets native
Windows SimConnect clients only.
