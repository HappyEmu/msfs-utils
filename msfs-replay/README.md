# msfs-replay

Shared timestamped-pose loading and interpolation for the async and blocking
SimConnect replay examples.

Recordings use CSV with an optional header:

```text
timestamp_seconds,latitude_degrees,longitude_degrees,altitude_feet,pitch_degrees,bank_degrees,heading_degrees
0.000,47.4580,8.5555,1416.0,0.0,0.0,350.0
0.050,47.4581,8.5557,1418.0,1.0,2.0,10.0
```

Timestamps must be finite and strictly increasing. They are normalized to
start at zero when the recording is loaded.

Latitude and altitude use linear interpolation. Longitude follows the shortest
wrapped path across the antimeridian. Pitch, bank, and heading are converted to
quaternions and interpolated with spherical linear interpolation, avoiding
independent Euler-angle wrapping and most attitude interpolation artifacts.
