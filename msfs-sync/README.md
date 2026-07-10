# msfs-sync

A blocking facade over the event-driven `msfs-async` SimConnect driver.

```rust
let sim = SimConnect::open("SYNC CLIENT")?;
let updates = sim.subscribe::<AircraftData>(
    SIMCONNECT_OBJECT_ID_USER,
    Period::SimFrame,
)?;

for value in updates {
    println!("{:?}", value?);
}
```

The driver still waits on the native Windows SimConnect event and drains the
queue automatically. Blocking consumers do not call `call_dispatch` or sleep
to poll for messages.

The same target-object controls are available synchronously:

```rust
sim.release_ai_control(target_id)?;
sim.set_freeze(target_id, FreezeState::ALL)?;
```

## Examples

- `request_once` performs blocking one-shot typed reads.
- `log` consumes several typed subscriptions on scoped threads.
- `client_data` ports the client-data writer and reader.
- `read_write_same_client` reads and writes through one cloned client handle.
- `multiplexed` maps several data types into one blocking event receiver on the
  application thread.
- `multiple_receivers` shows multiple independent receivers consumed correctly
  on separate threads, alongside a sequential-blocking anti-pattern.
- `relay_simobject` copies live, SimFrame-paced position and attitude from one
  explicit SimObject ID to another.
- `replay_simobject` reads timestamped poses from CSV and resamples them at a
  fixed output rate, using wrapped longitude interpolation and quaternion slerp
  for attitude.

The replay CSV columns and optional playback arguments match the async example:

```console
cargo run -p msfs-sync --example sync_replay_simobject -- flight.csv 42 60 1
```

Run an example on Windows with the MSFS SDK installed:

```console
cargo run -p msfs-sync --example sync_log
```
