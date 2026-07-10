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
cargo run -p msfs-multiplayer -- client 127.0.0.1:9997 "A320neo V2"
```

Publish ten synthetic users from a recording:

```console
cargo run -p msfs-multiplayer -- replay 127.0.0.1:9997 flight_a320_vel2.csv 10
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
Replay clients preserve the original `fsmp` recording's world and body
velocities. The SimConnect injector leaves released remote aircraft unfrozen,
sets their recorded attitude and body velocities, and lets the simulator move
them without subsequent latitude, longitude, or altitude corrections.
The relay currently encodes one complete snapshot per recipient every 33 ms,
making its snapshot work quadratic in the number of connected users; it is
intended for small prototype sessions, not large deployments.

This remains a prototype. It has no authentication, congestion control, model
matching, clock synchronization, or interpolation buffer. Every remote user is
rendered with the container title supplied to `client`.
