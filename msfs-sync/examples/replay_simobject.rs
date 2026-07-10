#[cfg(windows)]
mod windows {
    use msfs_replay::{Pose, Recording};
    use msfs_sync::{FreezeState, SimConnect, data_definition};
    use std::io;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    #[data_definition]
    #[derive(Debug)]
    struct FlightData {
        #[name = "PLANE LATITUDE"]
        #[unit = "Degrees"]
        latitude: f64,
        #[name = "PLANE LONGITUDE"]
        #[unit = "Degrees"]
        longitude: f64,
        #[name = "PLANE ALTITUDE"]
        #[unit = "Feet"]
        altitude: f64,
        #[name = "PLANE PITCH DEGREES"]
        #[unit = "Degrees"]
        pitch: f64,
        #[name = "PLANE BANK DEGREES"]
        #[unit = "Degrees"]
        bank: f64,
        #[name = "PLANE HEADING DEGREES TRUE"]
        #[unit = "Degrees"]
        heading: f64,
    }

    impl From<Pose> for FlightData {
        fn from(pose: Pose) -> Self {
            Self {
                latitude: pose.latitude,
                longitude: pose.longitude,
                altitude: pose.altitude,
                pitch: pose.pitch,
                bank: pose.bank,
                heading: pose.heading,
            }
        }
    }

    struct Args {
        recording: PathBuf,
        target_id: u32,
        rate_hz: f64,
        speed: f64,
    }

    pub fn main() -> Result<(), Box<dyn std::error::Error>> {
        let args = args()?;
        let recording = Recording::from_csv_path(&args.recording)?;
        eprintln!(
            "Replaying {:.3}s from {} onto object {} at {} Hz and {}x speed.",
            recording.duration_seconds(),
            args.recording.display(),
            args.target_id,
            args.rate_hz,
            args.speed,
        );

        let sim = SimConnect::open("SYNC TIMESTAMPED SIMOBJECT REPLAY")?;
        let mut exceptions = sim.exceptions()?;
        sim.release_ai_control(args.target_id)?;
        sim.set_freeze(args.target_id, FreezeState::ALL)?;

        let period_seconds = args.rate_hz.recip();
        let started = Instant::now();
        loop {
            let playback_seconds = started.elapsed().as_secs_f64() * args.speed;
            let finished = playback_seconds >= recording.duration_seconds();
            let pose = recording
                .sample(playback_seconds.min(recording.duration_seconds()))
                .expect("a validated recording always has a final sample");
            sim.set_data_on_sim_object(args.target_id, &FlightData::from(pose))?;

            if let Some(exception) = exceptions.try_recv() {
                return Err(Box::new(exception));
            }
            if finished {
                break;
            }

            sleep_until_next_tick(started, period_seconds);
        }

        sim.set_freeze(args.target_id, FreezeState::NONE)?;
        Ok(())
    }

    fn sleep_until_next_tick(started: Instant, period_seconds: f64) {
        let elapsed = started.elapsed().as_secs_f64();
        let next_tick = (elapsed / period_seconds).floor() + 1.0;
        let deadline = started + Duration::from_secs_f64(next_tick * period_seconds);
        std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
    }

    fn args() -> Result<Args, Box<dyn std::error::Error>> {
        let mut values = std::env::args().skip(1);
        let recording = values.next().ok_or_else(usage_error)?.into();
        let target_id = values.next().ok_or_else(usage_error)?.parse()?;
        let rate_hz = values
            .next()
            .map(|value| value.parse::<f64>())
            .transpose()?
            .unwrap_or(60.0);
        let speed = values
            .next()
            .map(|value| value.parse::<f64>())
            .transpose()?
            .unwrap_or(1.0);
        if values.next().is_some()
            || !rate_hz.is_finite()
            || rate_hz <= 0.0
            || !speed.is_finite()
            || speed <= 0.0
        {
            return Err(usage_error().into());
        }
        Ok(Args {
            recording,
            target_id,
            rate_hz,
            speed,
        })
    }

    fn usage_error() -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: sync_replay_simobject <recording.csv> <target-object-id> [rate-hz=60] [speed=1]",
        )
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("this example requires Windows and the Microsoft Flight Simulator SDK");
}
