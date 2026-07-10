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
the rendered motion and buffer occupancy can be inspected or plotted.

The relay uses versioned UDP datagrams, rejects stale per-user updates, excludes
the receiving user from snapshots, and expires silent clients after ten
seconds. The live client buffers every accepted update per remote user,
estimates sender clock offset, and renders a 30 Hz timeline with a fixed 500 ms
playout delay. Only the newest rendered snapshot crosses into the SimConnect
worker, so slow operations do not build an unbounded queue of stale poses.
Replay clients preserve the original `fsmp` recording's world and body
velocities. The SimConnect injector leaves released remote aircraft unfrozen,
creates them on a dedicated worker, and aligns each assigned object to a fresh
buffered target after its position, altitude, and attitude freeze states are
confirmed. It activates velocity-only playback after confirming those states
are unfrozen again, with no subsequent latitude, longitude, or altitude
corrections during normal operation. For the first 30 simulator seconds after
assignment, a bounded initialization watchdog repeats that confirmed alignment
if MSFS resets the object by more than 100 feet while loading its model. Each
reset extends monitoring until the object has remained stable for ten seconds.
For each remote user, the live client reads position and simulator absolute time
through a non-blocking once-per-second subscription. It aligns that measurement
with buffered target history on the simulator clock, then logs along-track,
cross-track, signed vertical, and total 3D error. Positive along-track means
ahead of the expected course; positive cross-track means to the right.
The relay currently encodes one complete snapshot per recipient every 33 ms,
making its snapshot work quadratic in the number of connected users; it is
intended for small prototype sessions, not large deployments.

This remains a prototype. It has no authentication, congestion control, or
model matching. Every remote user is rendered with the container title supplied
to `client`.
