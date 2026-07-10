# msfs-replay

Shared timestamped-pose loading and interpolation for the async and blocking
SimConnect replay examples.

Recordings use the headerless 13-column CSV format emitted by the original
`fsmp` prototype:

```text
elapsed_seconds,latitude_degrees,longitude_degrees,altitude_feet,velocity_world_x_fps,velocity_world_y_fps,velocity_world_z_fps,pitch_degrees,bank_degrees,heading_degrees,velocity_body_x_fps,velocity_body_y_fps,velocity_body_z_fps
0.000,47.4580,8.5555,1416.0,0.0,0.0,0.0,0.0,0.0,350.0,0.0,0.0,0.0
0.050,47.4581,8.5557,1418.0,1.0,2.0,3.0,1.0,2.0,10.0,4.0,5.0,6.0
```

The descriptive first line above is not part of the file. Recordings contain
only numeric rows, so existing `flight_a320_vel2.csv` files can be used without
conversion.

Timestamps must be finite and strictly increasing. They are normalized to
start at zero when the recording is loaded.

Latitude and altitude use linear interpolation. Longitude follows the shortest
wrapped path across the antimeridian. Pitch, bank, and heading are converted to
quaternions and interpolated with spherical linear interpolation, avoiding
independent Euler-angle wrapping and most attitude interpolation artifacts.
World and body velocities are interpolated linearly.
