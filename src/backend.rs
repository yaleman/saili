use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use saili::{EspHomeRcClient, RcChannels, ServerIdentity};
use thiserror::Error;

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(3);
const ACTION_TIMEOUT: Duration = Duration::from_millis(200);
const RECONNECT_DELAY: Duration = Duration::from_secs(1);
const INPUT_STALE_AFTER: Duration = Duration::from_millis(150);

#[derive(Clone, Debug)]
pub struct BackendConfig {
    pub address: String,
    pub encryption_key: String,
    pub transmit_interval: Duration,
}

pub struct Backend {
    desired: Arc<Mutex<DesiredOutput>>,
    stop: Arc<AtomicBool>,
    events: Receiver<BackendEvent>,
    thread: Option<JoinHandle<()>>,
}

impl Backend {
    pub fn start(config: BackendConfig) -> Result<Self, BackendError> {
        let desired = Arc::new(Mutex::new(DesiredOutput::safe()));
        let stop = Arc::new(AtomicBool::new(false));
        let (event_sender, events) = mpsc::channel();

        let worker_desired = Arc::clone(&desired);
        let worker_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("esphome-rc".to_owned())
            .spawn(move || run_worker(config, worker_desired, worker_stop, event_sender))
            .map_err(BackendError::Spawn)?;

        Ok(Self {
            desired,
            stop,
            events,
            thread: Some(thread),
        })
    }

    pub fn set_output(&self, mode: OutputMode, channels: RcChannels) {
        let mut desired = lock_recover(&self.desired);
        *desired = DesiredOutput {
            mode,
            channels,
            updated_at: Instant::now(),
        };
    }

    pub fn drain_events(&self) -> impl Iterator<Item = BackendEvent> + '_ {
        self.events.try_iter()
    }

    pub fn shutdown(mut self) -> Result<(), BackendError> {
        self.set_output(OutputMode::SafeHold, RcChannels::safe());
        self.stop.store(true, Ordering::Release);

        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| BackendError::WorkerPanicked)?;
        }
        Ok(())
    }
}

fn run_worker(
    config: BackendConfig,
    desired: Arc<Mutex<DesiredOutput>>,
    stop: Arc<AtomicBool>,
    events: Sender<BackendEvent>,
) {
    while !stop.load(Ordering::Acquire) {
        if events
            .send(BackendEvent::Connecting {
                address: config.address.clone(),
            })
            .is_err()
        {
            return;
        }

        match EspHomeRcClient::connect(&config.address, &config.encryption_key, CONNECTION_TIMEOUT)
        {
            Ok(mut client) => {
                if events
                    .send(BackendEvent::Connected {
                        server: client.server().clone(),
                    })
                    .is_err()
                {
                    return;
                }

                let mut sent = 0_u64;
                let mut acknowledged = 0_u64;
                let mut next_transmit = Instant::now();

                while !stop.load(Ordering::Acquire) {
                    let snapshot = *lock_recover(&desired);
                    let safe_override = snapshot.mode == OutputMode::SafeHold
                        || snapshot.updated_at.elapsed() > INPUT_STALE_AFTER;
                    let channels = if safe_override {
                        RcChannels::safe()
                    } else {
                        snapshot.channels
                    };

                    sent = sent.saturating_add(1);
                    match client.send_channels(channels, ACTION_TIMEOUT) {
                        Ok(acknowledgement) => {
                            acknowledged = acknowledged.saturating_add(1);
                            if events
                                .send(BackendEvent::CommandAcknowledged {
                                    sent,
                                    acknowledged,
                                    round_trip: acknowledgement.round_trip,
                                    safe_override,
                                })
                                .is_err()
                            {
                                return;
                            }
                        }
                        Err(error) => {
                            let _ = events.send(BackendEvent::Failed {
                                message: error.to_string(),
                            });
                            break;
                        }
                    }

                    next_transmit += config.transmit_interval;
                    sleep_until(next_transmit, &stop);
                    if next_transmit < Instant::now() {
                        next_transmit = Instant::now();
                    }
                }

                let _ = client.send_channels(RcChannels::safe(), ACTION_TIMEOUT);
            }
            Err(error) => {
                if events
                    .send(BackendEvent::Failed {
                        message: error.to_string(),
                    })
                    .is_err()
                {
                    return;
                }
            }
        }

        sleep_for(RECONNECT_DELAY, &stop);
    }
}

fn sleep_until(deadline: Instant, stop: &AtomicBool) {
    while !stop.load(Ordering::Acquire) {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        thread::sleep((deadline - now).min(Duration::from_millis(10)));
    }
}

fn sleep_for(duration: Duration, stop: &AtomicBool) {
    sleep_until(Instant::now() + duration, stop);
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputMode {
    SafeHold,
    Live,
}

#[derive(Clone, Copy)]
struct DesiredOutput {
    mode: OutputMode,
    channels: RcChannels,
    updated_at: Instant,
}

impl DesiredOutput {
    fn safe() -> Self {
        Self {
            mode: OutputMode::SafeHold,
            channels: RcChannels::safe(),
            updated_at: Instant::now(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum BackendEvent {
    Connecting {
        address: String,
    },
    Connected {
        server: ServerIdentity,
    },
    CommandAcknowledged {
        sent: u64,
        acknowledged: u64,
        round_trip: Duration,
        safe_override: bool,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("could not start ESPHome network worker")]
    Spawn(#[source] std::io::Error),

    #[error("ESPHome network worker panicked")]
    WorkerPanicked,
}
