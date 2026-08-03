use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{
    DecodedState, Decoder, DecoderStatus, MuxLossReason, MuxState, RawReport, ReadReportStatus,
    ReportFormat, SailiDevice,
};

const READ_TIMEOUT: Duration = Duration::from_millis(10);
const RECONNECT_DELAY: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatConfidence {
    Explicit,
    Metadata,
    Uncertain,
}

impl FormatConfidence {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Metadata => "metadata",
            Self::Uncertain => "uncertain",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReaderConfig {
    pub report_format: ReportFormat,
    pub swap_mux_channels: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReaderCommand {
    StartMuxCalibration,
    ConfirmMuxCalibration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ReaderCommandError {
    #[error("reader command queue is full")]
    QueueFull,
    #[error("reader thread is no longer running")]
    Disconnected,
}

#[derive(Debug, thiserror::Error)]
pub enum ReaderStartError {
    #[error("could not start HID reader thread")]
    Spawn(#[source] std::io::Error),
}

#[derive(Clone, Debug)]
pub struct ReaderStats {
    pub reports_received: u64,
    pub decoded_states: u64,
    pub malformed_reports: u64,
    pub timeouts: u64,
    pub reconnects: u64,
    pub read_errors: u64,
    pub mux_loss_events: u64,
    pub coalesced_updates: u64,
    pub last_error: Option<String>,
    pub last_raw_at: Option<Instant>,
    pub last_complete_at: Option<Instant>,
    pub selected_format: Option<ReportFormat>,
    pub format_confidence: FormatConfidence,
    pub selection_reason: String,
    pub decoder_status: DecoderStatus,
}

impl ReaderStats {
    fn new(format: ReportFormat, confidence: FormatConfidence, selection_reason: String) -> Self {
        Self {
            reports_received: 0,
            decoded_states: 0,
            malformed_reports: 0,
            timeouts: 0,
            reconnects: 0,
            read_errors: 0,
            mux_loss_events: 0,
            coalesced_updates: 0,
            last_error: None,
            last_raw_at: None,
            last_complete_at: None,
            selected_format: (format != ReportFormat::Auto).then_some(format),
            format_confidence: confidence,
            selection_reason,
            decoder_status: Decoder::new(format, false).status(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReaderSnapshot {
    pub revision: u64,
    pub latest_raw: Option<RawReport>,
    pub state: Option<DecodedState>,
    pub stats: ReaderStats,
}

impl ReaderSnapshot {
    fn new(format: ReportFormat, confidence: FormatConfidence, selection_reason: String) -> Self {
        Self {
            revision: 0,
            latest_raw: None,
            state: None,
            stats: ReaderStats::new(format, confidence, selection_reason),
        }
    }

    #[must_use]
    pub fn complete_state_is_fresh(&self, now: Instant, maximum_age: Duration) -> bool {
        self.stats
            .last_complete_at
            .is_some_and(|updated| now.saturating_duration_since(updated) <= maximum_age)
    }

    #[must_use]
    pub fn live_input_is_ready(&self, now: Instant, maximum_age: Duration) -> bool {
        self.state.is_some()
            && self.complete_state_is_fresh(now, maximum_age)
            && self
                .stats
                .decoder_status
                .mux_state
                .is_none_or(|state| state == MuxState::Calibrated)
    }
}

pub struct ReaderHandle {
    latest: Arc<Mutex<ReaderSnapshot>>,
    updates: Receiver<()>,
    commands: SyncSender<ReaderCommand>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

struct ReaderRuntime {
    latest: Arc<Mutex<ReaderSnapshot>>,
    updates: SyncSender<()>,
    commands: Receiver<ReaderCommand>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl ReaderHandle {
    #[must_use]
    pub fn snapshot(&self) -> ReaderSnapshot {
        let _ = self.updates.try_iter().count();
        lock_recover(&self.latest).clone()
    }

    pub fn shutdown(mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    pub fn start_mux_calibration(&self) -> Result<(), ReaderCommandError> {
        self.send_command(ReaderCommand::StartMuxCalibration)
    }

    pub fn confirm_mux_calibration(&self) -> Result<(), ReaderCommandError> {
        self.send_command(ReaderCommand::ConfirmMuxCalibration)
    }

    fn send_command(&self, command: ReaderCommand) -> Result<(), ReaderCommandError> {
        match self.commands.try_send(command) {
            Ok(()) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Full(_)) => Err(ReaderCommandError::QueueFull),
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                Err(ReaderCommandError::Disconnected)
            }
        }
    }
}

impl Drop for ReaderHandle {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl SailiDevice {
    pub fn spawn_reader(self, config: ReaderConfig) -> Result<ReaderHandle, ReaderStartError> {
        let (selected_format, confidence, selection_reason) = match config.report_format {
            ReportFormat::Auto => self.identity.format_hint().map_or(
                (
                    ReportFormat::Auto,
                    FormatConfidence::Uncertain,
                    "no safe format hint was available".to_owned(),
                ),
                |format| {
                    (
                        format,
                        FormatConfidence::Metadata,
                        format!("device metadata selected {format}"),
                    )
                },
            ),
            format => (
                format,
                FormatConfidence::Explicit,
                format!("explicit --report-format {format}"),
            ),
        };
        let latest = Arc::new(Mutex::new(ReaderSnapshot::new(
            selected_format,
            confidence,
            selection_reason,
        )));
        let (updates_sender, updates) = mpsc::sync_channel(1);
        let (commands, commands_receiver) = mpsc::sync_channel(4);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_latest = Arc::clone(&latest);
        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("saili-hid-reader".to_owned())
            .spawn(move || {
                run_reader(
                    self,
                    selected_format,
                    confidence,
                    config.swap_mux_channels,
                    ReaderRuntime {
                        latest: thread_latest,
                        updates: updates_sender,
                        commands: commands_receiver,
                        stop: thread_stop,
                    },
                );
            })
            .map_err(ReaderStartError::Spawn)?;

        Ok(ReaderHandle {
            latest,
            updates,
            commands,
            stop,
            thread: Some(thread),
        })
    }
}

fn run_reader(
    mut device: SailiDevice,
    format: ReportFormat,
    confidence: FormatConfidence,
    swap_mux_channels: bool,
    runtime: ReaderRuntime,
) {
    let ReaderRuntime {
        latest,
        updates,
        commands,
        stop,
    } = runtime;
    let mut decoder = Decoder::new(format, swap_mux_channels);
    let mut sequence = 0_u64;
    let mut previous_raw = None;
    publish(&latest, &updates, |snapshot| {
        snapshot.stats.decoder_status = decoder.status();
    });

    while !stop.load(std::sync::atomic::Ordering::Acquire) {
        for command in commands.try_iter() {
            let command_result = match command {
                ReaderCommand::StartMuxCalibration => {
                    let result = decoder.start_mux_calibration();
                    if result.is_ok() {
                        previous_raw = None;
                    }
                    result
                }
                ReaderCommand::ConfirmMuxCalibration => decoder.confirm_mux_calibration(),
            };
            publish(&latest, &updates, |snapshot| {
                snapshot.revision = snapshot.revision.saturating_add(1);
                snapshot.stats.decoder_status = decoder.status();
                if let Err(error) = command_result {
                    snapshot.stats.last_error = Some(error.to_string());
                } else {
                    snapshot.state = None;
                    snapshot.stats.last_complete_at = None;
                    snapshot.stats.last_error = None;
                }
            });
        }

        match device.read_report(READ_TIMEOUT) {
            Ok(ReadReportStatus::Timeout) => {
                publish(&latest, &updates, |snapshot| {
                    snapshot.stats.timeouts = snapshot.stats.timeouts.saturating_add(1);
                });
            }
            Ok(ReadReportStatus::Report { bytes, received_at }) => {
                sequence = sequence.saturating_add(1);
                publish(&latest, &updates, |snapshot| {
                    snapshot.stats.reports_received =
                        snapshot.stats.reports_received.saturating_add(1);
                    snapshot.stats.last_raw_at = Some(received_at);
                });
                let raw = match RawReport::try_new(
                    &bytes,
                    sequence,
                    received_at,
                    previous_raw.as_ref(),
                ) {
                    Ok(raw) => raw,
                    Err(error) => {
                        decoder.mark_lost(MuxLossReason::MalformedReport);
                        previous_raw = None;
                        publish(&latest, &updates, |snapshot| {
                            snapshot.revision = snapshot.revision.saturating_add(1);
                            snapshot.state = None;
                            snapshot.stats.last_complete_at = None;
                            snapshot.stats.decoder_status = decoder.status();
                            snapshot.stats.mux_loss_events =
                                snapshot.stats.mux_loss_events.saturating_add(1);
                            snapshot.stats.malformed_reports =
                                snapshot.stats.malformed_reports.saturating_add(1);
                            snapshot.stats.last_error = Some(error.to_string());
                        });
                        continue;
                    }
                };
                previous_raw = Some(raw);
                let was_lost = decoder.status().mux_state == Some(MuxState::Lost);
                let decoded = decoder.decode(raw);
                let decoder_status = decoder.status();
                let became_lost = !was_lost && decoder_status.mux_state == Some(MuxState::Lost);
                if became_lost {
                    previous_raw = None;
                }
                publish(&latest, &updates, |snapshot| {
                    snapshot.revision = snapshot.revision.saturating_add(1);
                    snapshot.latest_raw = Some(raw);
                    snapshot.stats.last_raw_at = Some(raw.received_at());
                    snapshot.stats.decoder_status = decoder_status;
                    if became_lost {
                        snapshot.state = None;
                        snapshot.stats.last_complete_at = None;
                        snapshot.stats.mux_loss_events =
                            snapshot.stats.mux_loss_events.saturating_add(1);
                    }
                    match decoded {
                        Ok(Some(state)) => {
                            if decoder_status.mux_state != Some(MuxState::Lost) {
                                snapshot.state = Some(state);
                                snapshot.stats.decoded_states =
                                    snapshot.stats.decoded_states.saturating_add(1);
                                snapshot.stats.last_complete_at = Some(raw.received_at());
                            }
                        }
                        Ok(None) => {}
                        Err(error) => snapshot.stats.last_error = Some(error.to_string()),
                    }
                });
            }
            Err(error) => {
                decoder.mark_lost(MuxLossReason::ReadError);
                publish(&latest, &updates, |snapshot| {
                    snapshot.revision = snapshot.revision.saturating_add(1);
                    snapshot.state = None;
                    snapshot.latest_raw = None;
                    snapshot.stats.last_complete_at = None;
                    snapshot.stats.read_errors = snapshot.stats.read_errors.saturating_add(1);
                    snapshot.stats.mux_loss_events =
                        snapshot.stats.mux_loss_events.saturating_add(1);
                    snapshot.stats.decoder_status = decoder.status();
                    snapshot.stats.last_error = Some(error.to_string());
                });
                decoder.reset();
                previous_raw = None;
                reconnect(&mut device, &latest, &updates, &stop);
                if !stop.load(std::sync::atomic::Ordering::Acquire) {
                    decoder.reset();
                    previous_raw = None;
                    if format == ReportFormat::RawMuxed8 {
                        let _ = decoder.start_mux_calibration();
                    }
                    publish(&latest, &updates, |snapshot| {
                        snapshot.revision = snapshot.revision.saturating_add(1);
                        snapshot.state = None;
                        snapshot.stats.last_complete_at = None;
                        snapshot.stats.decoder_status = decoder.status();
                    });
                }
            }
        }
    }

    publish(&latest, &updates, |snapshot| {
        snapshot.state = None;
        snapshot.latest_raw = None;
        snapshot.stats.decoder_status = decoder.status();
        snapshot.stats.format_confidence = confidence;
    });
}

fn reconnect(
    device: &mut SailiDevice,
    latest: &Arc<Mutex<ReaderSnapshot>>,
    updates: &SyncSender<()>,
    stop: &std::sync::atomic::AtomicBool,
) {
    while !stop.load(std::sync::atomic::Ordering::Acquire) {
        match device.reconnect() {
            Ok(()) => {
                publish(latest, updates, |snapshot| {
                    snapshot.revision = snapshot.revision.saturating_add(1);
                    snapshot.stats.reconnects = snapshot.stats.reconnects.saturating_add(1);
                });
                return;
            }
            Err(error) => {
                publish(latest, updates, |snapshot| {
                    snapshot.stats.last_error = Some(error.to_string());
                });
                thread::sleep(RECONNECT_DELAY);
            }
        }
    }
}

fn publish<F>(latest: &Arc<Mutex<ReaderSnapshot>>, updates: &SyncSender<()>, update: F)
where
    F: FnOnce(&mut ReaderSnapshot),
{
    let mut snapshot = lock_recover(latest);
    update(&mut snapshot);
    drop(snapshot);
    match updates.try_send(()) {
        Ok(()) | Err(TrySendError::Disconnected(())) => {}
        Err(TrySendError::Full(())) => {
            let mut snapshot = lock_recover(latest);
            snapshot.stats.coalesced_updates = snapshot.stats.coalesced_updates.saturating_add(1);
        }
    }
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_snapshot_coalesces_updates_without_blocking_reader() {
        let latest = Arc::new(Mutex::new(ReaderSnapshot::new(
            ReportFormat::LinuxDemuxed8,
            FormatConfidence::Explicit,
            "test format".to_owned(),
        )));
        let (sender, receiver) = mpsc::sync_channel(1);
        publish(&latest, &sender, |snapshot| snapshot.revision = 1);
        publish(&latest, &sender, |snapshot| snapshot.revision = 2);
        assert_eq!(lock_recover(&latest).revision, 2);
        assert_eq!(lock_recover(&latest).stats.coalesced_updates, 1);
        assert_eq!(receiver.try_iter().count(), 1);
    }

    #[test]
    fn incomplete_or_old_state_is_not_fresh() {
        let mut snapshot = ReaderSnapshot::new(
            ReportFormat::RawMuxed8,
            FormatConfidence::Explicit,
            "test format".to_owned(),
        );
        let now = Instant::now();
        snapshot.stats.last_complete_at = Some(now - Duration::from_millis(151));
        assert!(!snapshot.complete_state_is_fresh(now, Duration::from_millis(150)));
        snapshot.stats.last_complete_at = None;
        assert!(!snapshot.complete_state_is_fresh(now, Duration::from_millis(150)));
    }
}
