# msfs-multiplayer

An experimental multiplayer state relay and native SimConnect injector inspired
by the earlier `fsmp` prototype.

The binary has three modes:

- `server` runs a platform-independent UDP state relay;
- `client` publishes the user's aircraft and injects remote users through
  `msfs-sync` on Windows;
- `replay` publishes one or more `msfs-replay` recordings as synthetic users.

Start a relay:

```console
cargo run -p msfs-multiplayer -- server 0.0.0.0:9997
```

Connect MSFS using an installed aircraft container title for remote users:

```console
cargo run -p msfs-multiplayer -- client 127.0.0.1:9997 "Airbus A320 Neo Asobo"
```

Publish ten synthetic users from a recording:

```console
cargo run -p msfs-multiplayer -- replay 127.0.0.1:9997 flight.csv 10
```

Run the platform-independent jitter-buffer example:

```console
cargo run -p msfs-multiplayer --example interpolation
```

The example generates 10 Hz sender samples with variable arrival delay and
resamples them at 60 Hz through a 150 ms interpolation buffer. It prints CSV so
the rendered motion and buffer occupancy can be inspected or plotted. The live
client does not use this buffer yet.

The relay uses versioned UDP datagrams, rejects stale per-user updates, excludes
the receiving user from snapshots, and expires silent clients after ten
seconds. The live client shares only the newest local state and remote snapshot,
so slow SimConnect operations do not build an unbounded queue of stale poses.

This remains a prototype. It has no authentication, congestion control, model
matching, clock synchronization, or interpolation buffer. Every remote user is
rendered with the container title supplied to `client`.
