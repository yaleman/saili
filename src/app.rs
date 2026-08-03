use std::io;
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Gauge, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use saili::{
    ArmConfig, ArmController, CHANNEL_COUNT, CrsfFrame, CrsfTelemetry, DecodedState,
    DeviceIdentity, MappingError, MuxState, RC_MAX_US, RC_MIN_US, RcChannels, RcMapping,
    ReaderConfig, ReaderHandle, ReaderSnapshot, ReaderStartError, ReportFormat, SailiDevice,
    SailiError, ServerIdentity,
};
use thiserror::Error;

use crate::backend::{Backend, BackendConfig, BackendError, BackendEvent, OutputMode};

const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(40);
const LIVE_INPUT_MAX_AGE: Duration = Duration::from_millis(150);
const SAFE_THROTTLE_MAX_US: u16 = 1050;
const DEFAULT_TRANSMIT_RATE_HZ: u16 = 20;
const MAX_TRANSMIT_RATE_HZ: u16 = 50;

pub fn run() -> Result<(), AppError> {
    let arguments = Arguments::parse();
    let config = RuntimeConfig::try_from(arguments)?;
    let (device, device_notice) = match SailiDevice::connect() {
        Ok(device) => (Some(device), None),
        Err(error) => (
            None,
            Some(format!(
                "SAILI input unavailable: {error}; configuration remains available"
            )),
        ),
    };
    let identity = device.as_ref().map(|current| current.identity().clone());
    let needs_mux_calibration =
        identity
            .as_ref()
            .is_some_and(|identity| match config.reader.report_format {
                ReportFormat::RawMuxed8 => true,
                ReportFormat::Auto => identity.format_hint() == Some(ReportFormat::RawMuxed8),
                ReportFormat::LinuxDemuxed8 | ReportFormat::Legacy7Button => false,
            });
    let mut app = App::new(identity, config.mapping, config.arm);
    if let Some(notice) = device_notice {
        app.notice = notice;
    }
    let reader = device
        .map(|device| device.spawn_reader(config.reader))
        .transpose()?;
    if needs_mux_calibration {
        if let Some(reader) = reader.as_ref() {
            let _ = reader.start_mux_calibration();
        }
        app.notice =
            "Raw mux input requires calibration: move input 7, press Enter, then move input 8"
                .to_owned();
    }
    let backend = match Backend::start(config.backend) {
        Ok(backend) => backend,
        Err(error) => {
            if let Some(reader) = reader {
                reader.shutdown();
            }
            return Err(error.into());
        }
    };
    let mut terminal = match ratatui::try_init() {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = backend.shutdown();
            if let Some(reader) = reader {
                reader.shutdown();
            }
            return Err(AppError::Terminal(error));
        }
    };

    let run_result = terminal
        .clear()
        .map_err(AppError::Terminal)
        .and_then(|()| run_loop(&mut terminal, reader.as_ref(), &backend, &mut app));
    if let Some(reader) = reader {
        reader.shutdown();
    }
    let shutdown_result = backend.shutdown();
    ratatui::restore();

    run_result?;
    shutdown_result?;
    Ok(())
}

fn run_loop(
    terminal: &mut DefaultTerminal,
    reader: Option<&ReaderHandle>,
    backend: &Backend,
    app: &mut App,
) -> Result<(), AppError> {
    loop {
        if let Some(reader) = reader {
            let snapshot = reader.snapshot();
            if snapshot.revision != app.reader_revision {
                app.update_reader(snapshot);
                backend.set_output(app.output_mode, app.mapped_input);
            }
        }

        for backend_event in backend.drain_events() {
            app.update_backend(backend_event);
        }

        terminal
            .draw(|frame| render(frame, app))
            .map_err(AppError::Terminal)?;

        if event::poll(INPUT_POLL_INTERVAL).map_err(AppError::Input)? {
            match handle_event(event::read().map_err(AppError::Input)?, app, reader) {
                AppCommand::Continue => {
                    backend.set_output(app.output_mode, app.mapped_input);
                }
                AppCommand::Quit => {
                    app.output_mode = OutputMode::SafeHold;
                    backend.set_output(OutputMode::SafeHold, RcChannels::safe());
                    return Ok(());
                }
            }
        }
    }
}

fn handle_event(event: Event, app: &mut App, reader: Option<&ReaderHandle>) -> AppCommand {
    let Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        ..
    }) = event
    else {
        return AppCommand::Continue;
    };

    if let Some(editor) = app.mapping_editor.as_mut() {
        let action = handle_mapping_event(code, editor);
        match action {
            MappingEditorAction::Cancel => {
                app.mapping_editor = None;
                app.notice = "Mapping changes cancelled; output remains in safe hold".to_owned();
            }
            MappingEditorAction::Save => {
                if let Some(editor) = app.mapping_editor.take() {
                    app.apply_mapping(editor);
                }
            }
            MappingEditorAction::Continue => {}
        }
        return AppCommand::Continue;
    }

    if matches!(code, KeyCode::Esc | KeyCode::Char('q'))
        || (code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL))
    {
        return AppCommand::Quit;
    }

    if code == KeyCode::Char('l') {
        app.toggle_live();
    }
    if code == KeyCode::Char('m') {
        app.open_mapping_editor();
    }
    if code == KeyCode::Char('p') {
        app.start_mux_calibration(reader);
    }
    if code == KeyCode::Enter && app.mux_calibration_in_progress() {
        app.confirm_mux_calibration(reader);
    }

    AppCommand::Continue
}

enum AppCommand {
    Continue,
    Quit,
}

struct App {
    identity: Option<DeviceIdentity>,
    mapping: RcMapping,
    arm: ArmController,
    state: Option<DecodedState>,
    mapped_input: RcChannels,
    output_mode: OutputMode,
    last_update: Option<Instant>,
    reader_revision: u64,
    reader_snapshot: Option<ReaderSnapshot>,
    backend_state: BackendState,
    commands_sent: u64,
    commands_acknowledged: u64,
    last_round_trip: Option<Duration>,
    safe_override: bool,
    telemetry: FlightControllerTelemetry,
    notice: String,
    mapping_editor: Option<MappingEditor>,
}

impl App {
    fn new(identity: Option<DeviceIdentity>, mapping: RcMapping, arm: ArmConfig) -> Self {
        Self {
            identity,
            mapping,
            arm: ArmController::new(arm),
            state: None,
            mapped_input: RcChannels::safe(),
            output_mode: OutputMode::SafeHold,
            last_update: None,
            reader_revision: 0,
            reader_snapshot: None,
            backend_state: BackendState::Starting,
            commands_sent: 0,
            commands_acknowledged: 0,
            last_round_trip: None,
            safe_override: true,
            telemetry: FlightControllerTelemetry::default(),
            notice: "Safe hold active; press l with throttle low and arm off".to_owned(),
            mapping_editor: None,
        }
    }

    fn update_reader(&mut self, snapshot: ReaderSnapshot) {
        self.reader_revision = snapshot.revision;
        self.reader_snapshot = Some(snapshot.clone());
        if let Some(state) = snapshot.state {
            self.mapped_input = self.mapping.map(state).with_arm(self.arm.update(state));
            self.state = Some(state);
            self.last_update = Some(state.raw().received_at());
        } else {
            self.state = None;
            self.mapped_input = RcChannels::safe();
            self.arm.reset();
            self.last_update = None;
        }
    }

    fn mux_calibration_in_progress(&self) -> bool {
        self.reader_snapshot.as_ref().is_some_and(|snapshot| {
            matches!(
                snapshot.stats.decoder_status.mux_state,
                Some(MuxState::CalibratingFirst | MuxState::CalibratingSecond)
            )
        })
    }

    fn start_mux_calibration(&mut self, reader: Option<&ReaderHandle>) {
        self.output_mode = OutputMode::SafeHold;
        let Some(reader) = reader else {
            self.notice = "Cannot calibrate: HID reader is unavailable".to_owned();
            return;
        };
        match reader.start_mux_calibration() {
            Ok(()) => {
                self.notice =
                    "Move muxed input 7 through its range, then press Enter to confirm".to_owned();
            }
            Err(error) => self.notice = format!("Cannot start mux calibration: {error}"),
        }
    }

    fn confirm_mux_calibration(&mut self, reader: Option<&ReaderHandle>) {
        let Some(reader) = reader else {
            self.notice = "Cannot confirm calibration: HID reader is unavailable".to_owned();
            return;
        };
        match reader.confirm_mux_calibration() {
            Ok(()) => {
                self.notice = "Calibration step submitted; follow the next input prompt".to_owned();
            }
            Err(error) => self.notice = format!("Calibration rejected: {error}"),
        }
    }

    fn open_mapping_editor(&mut self) {
        self.output_mode = OutputMode::SafeHold;
        self.mapping_editor = Some(MappingEditor::from_mapping(self.mapping));
        self.notice = "Mapping editor open; output held safe".to_owned();
    }

    fn apply_mapping(&mut self, editor: MappingEditor) {
        match RcMapping::new_full(editor.channels, editor.inverted) {
            Ok(mapping) => {
                self.mapping = mapping;
                if let Some(state) = self.state {
                    self.mapped_input = self.mapping.map(state);
                }
                self.mapping_editor = None;
                self.notice = "Mapping saved; output remains in safe hold".to_owned();
            }
            Err(error) => {
                self.mapping_editor = Some(editor.with_error(error.to_string()));
            }
        }
    }

    fn update_backend(&mut self, event: BackendEvent) {
        match event {
            BackendEvent::Connecting { address } => {
                self.backend_state = BackendState::Connecting { address };
            }
            BackendEvent::Connected { server } => {
                self.backend_state = BackendState::Connected { server };
            }
            BackendEvent::CommandAcknowledged {
                sent,
                acknowledged,
                round_trip,
                safe_override,
            } => {
                self.commands_sent = sent;
                self.commands_acknowledged = acknowledged;
                self.last_round_trip = Some(round_trip);
                self.safe_override = safe_override;
            }
            BackendEvent::TelemetryReceived { frame } => {
                self.telemetry.update(frame);
            }
            BackendEvent::TelemetryRejected { message } => {
                self.telemetry.reject(message);
            }
            BackendEvent::Failed { message } => {
                self.backend_state = BackendState::Failed { message };
                self.safe_override = true;
            }
        }
    }

    fn toggle_live(&mut self) {
        if self.output_mode == OutputMode::Live {
            self.output_mode = OutputMode::SafeHold;
            self.notice = "Safe hold enabled".to_owned();
            return;
        }

        if !self.reader_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.live_input_is_ready(Instant::now(), LIVE_INPUT_MAX_AGE)
        }) {
            let message = if self.last_update.is_some() {
                "Cannot go live while controller input is stale"
            } else {
                "Cannot go live before the first complete controller report"
            };
            self.notice = message.to_owned();
            return;
        }
        if self.mapped_input.throttle() > SAFE_THROTTLE_MAX_US {
            self.notice = format!(
                "Cannot go live: throttle is {} us (must be <= {SAFE_THROTTLE_MAX_US})",
                self.mapped_input.throttle()
            );
            return;
        }
        if self.arm.is_armed() {
            self.notice = "Cannot go live while the configured arm source is on".to_owned();
            return;
        }
        self.output_mode = OutputMode::Live;
        self.notice = if self.arm.has_source() {
            "Live forwarding enabled; arm source controls CH5".to_owned()
        } else {
            "Live forwarding enabled; CH5 remains forced low".to_owned()
        };
    }
}

struct MappingEditor {
    selected: usize,
    channels: [usize; CHANNEL_COUNT],
    inverted: [bool; CHANNEL_COUNT],
    error: Option<String>,
}

impl MappingEditor {
    fn from_mapping(mapping: RcMapping) -> Self {
        Self {
            selected: 0,
            channels: mapping.all_channels(),
            inverted: mapping.inverted(),
            error: None,
        }
    }

    fn with_error(mut self, error: String) -> Self {
        self.error = Some(error);
        self
    }
}

enum MappingEditorAction {
    Continue,
    Cancel,
    Save,
}

fn handle_mapping_event(code: KeyCode, editor: &mut MappingEditor) -> MappingEditorAction {
    match code {
        KeyCode::Esc => return MappingEditorAction::Cancel,
        KeyCode::Up => {
            editor.selected = editor.selected.saturating_sub(1);
        }
        KeyCode::Down => {
            editor.selected = (editor.selected + 1).min(CHANNEL_COUNT - 1);
        }
        KeyCode::Left => {
            editor.channels[editor.selected] =
                editor.channels[editor.selected].saturating_sub(1).max(1);
            editor.error = None;
        }
        KeyCode::Right => {
            editor.channels[editor.selected] =
                (editor.channels[editor.selected] + 1).min(CHANNEL_COUNT);
            editor.error = None;
        }
        KeyCode::Char('i') => {
            editor.inverted[editor.selected] = !editor.inverted[editor.selected];
            editor.error = None;
        }
        KeyCode::Enter => return MappingEditorAction::Save,
        _ => {}
    }
    MappingEditorAction::Continue
}

enum BackendState {
    Starting,
    Connecting { address: String },
    Connected { server: ServerIdentity },
    Failed { message: String },
}

#[derive(Default)]
struct FlightControllerTelemetry {
    frames_received: u64,
    frames_rejected: u64,
    last_update: Option<Instant>,
    last_frame: Option<CrsfFrame>,
    last_kind: Option<&'static str>,
    last_error: Option<String>,
    battery: Option<BatteryTelemetry>,
    attitude: Option<AttitudeTelemetry>,
    gps: Option<GpsTelemetry>,
    flight_mode: Option<String>,
    vario_ms: Option<f32>,
    barometric_altitude_metres: Option<f32>,
    barometer: Option<BarometerTelemetry>,
    magnetometer: Option<MagnetometerTelemetry>,
    range: Option<RangeTelemetry>,
}

impl FlightControllerTelemetry {
    fn update(&mut self, frame: CrsfFrame) {
        self.frames_received = self.frames_received.saturating_add(1);
        self.last_update = Some(Instant::now());
        self.last_error = None;

        match frame.telemetry() {
            Ok(telemetry) => {
                self.last_kind = Some(telemetry.name());
                match telemetry {
                    CrsfTelemetry::Gps {
                        latitude_degrees,
                        longitude_degrees,
                        ground_speed_kmh,
                        heading_degrees,
                        altitude_metres,
                        satellites,
                    } => {
                        self.gps = Some(GpsTelemetry {
                            latitude_degrees,
                            longitude_degrees,
                            ground_speed_kmh,
                            heading_degrees,
                            altitude_metres,
                            satellites,
                        });
                    }
                    CrsfTelemetry::Vario { vertical_speed_ms } => {
                        self.vario_ms = Some(vertical_speed_ms);
                    }
                    CrsfTelemetry::Battery {
                        voltage_v,
                        current_a,
                        capacity_mah,
                        remaining_percent,
                    } => {
                        self.battery = Some(BatteryTelemetry {
                            voltage_v,
                            current_a,
                            capacity_mah,
                            remaining_percent,
                        });
                    }
                    CrsfTelemetry::BarometricAltitude {
                        altitude_metres,
                        vertical_speed_ms,
                    } => {
                        self.barometric_altitude_metres = Some(altitude_metres);
                        self.vario_ms = Some(vertical_speed_ms);
                    }
                    CrsfTelemetry::Barometer {
                        pressure_pa,
                        temperature_c,
                    } => {
                        self.barometer = Some(BarometerTelemetry {
                            pressure_pa,
                            temperature_c,
                        });
                    }
                    CrsfTelemetry::Magnetometer { x, y, z } => {
                        self.magnetometer = Some(MagnetometerTelemetry { x, y, z });
                    }
                    CrsfTelemetry::Attitude {
                        pitch_radians,
                        roll_radians,
                        yaw_radians,
                    } => {
                        self.attitude = Some(AttitudeTelemetry {
                            pitch_radians,
                            roll_radians,
                            yaw_radians,
                        });
                    }
                    CrsfTelemetry::FlightMode(mode) => {
                        self.flight_mode = Some(mode);
                    }
                    CrsfTelemetry::Range {
                        front_metres,
                        back_metres,
                        left_metres,
                        right_metres,
                    } => {
                        self.range = Some(RangeTelemetry {
                            front_metres,
                            back_metres,
                            left_metres,
                            right_metres,
                        });
                    }
                    CrsfTelemetry::Heartbeat
                    | CrsfTelemetry::DeviceInfo
                    | CrsfTelemetry::MspResponse
                    | CrsfTelemetry::Unknown { .. } => {}
                }
            }
            Err(error) => {
                self.frames_rejected = self.frames_rejected.saturating_add(1);
                self.last_kind = Some("Malformed");
                self.last_error = Some(error.to_string());
            }
        }

        self.last_frame = Some(frame);
    }

    fn reject(&mut self, message: String) {
        self.frames_rejected = self.frames_rejected.saturating_add(1);
        self.last_error = Some(message);
    }
}

struct BatteryTelemetry {
    voltage_v: f32,
    current_a: f32,
    capacity_mah: u32,
    remaining_percent: u8,
}

struct AttitudeTelemetry {
    pitch_radians: f32,
    roll_radians: f32,
    yaw_radians: f32,
}

struct GpsTelemetry {
    latitude_degrees: f64,
    longitude_degrees: f64,
    ground_speed_kmh: f32,
    heading_degrees: f32,
    altitude_metres: i32,
    satellites: u8,
}

struct BarometerTelemetry {
    pressure_pa: i32,
    temperature_c: f32,
}

struct MagnetometerTelemetry {
    x: i16,
    y: i16,
    z: i16,
}

struct RangeTelemetry {
    front_metres: Option<f32>,
    back_metres: Option<f32>,
    left_metres: Option<f32>,
    right_metres: Option<f32>,
}

fn render(frame: &mut Frame, app: &App) {
    let [
        status_area,
        channels_area,
        rc_area,
        backend_area,
        telemetry_area,
        raw_area,
        help_area,
    ] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length((CHANNEL_COUNT + 2) as u16),
        Constraint::Length(7),
        Constraint::Length(4),
        Constraint::Length(8),
        Constraint::Length(3),
        Constraint::Min(1),
    ])
    .areas(frame.area());

    render_status(frame, status_area, app);
    render_channels(frame, channels_area, app.state);
    render_rc_input(frame, rc_area, app);
    render_backend(frame, backend_area, app);
    render_telemetry(frame, telemetry_area, &app.telemetry);
    render_raw_packet(
        frame,
        raw_area,
        app.reader_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.latest_raw),
    );
    render_help(frame, help_area, app);
    if let Some(editor) = &app.mapping_editor {
        render_mapping_editor(frame, editor);
    }
}

fn render_mapping_editor(frame: &mut Frame, editor: &MappingEditor) {
    let area = centered_rect(60, 60, frame.area());
    frame.render_widget(Clear, area);

    let names = [
        "ROLL", "PITCH", "THROTTLE", "YAW", "AUX2", "AUX3", "AUX4", "AUX5",
    ];
    let mut lines = vec![Line::from("Select an input channel for each output")];
    lines.push(Line::from(""));
    for (index, name) in names.iter().enumerate() {
        let marker = if index == editor.selected { ">" } else { " " };
        let style = if index == editor.selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!(
                "{marker} {name:<8} CH{}  {}",
                editor.channels[index],
                if editor.inverted[index] {
                    "inverted"
                } else {
                    "normal"
                }
            ),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(
        "↑/↓ select   ←/→ input   i invert   Enter save   Esc cancel",
    ));
    if let Some(error) = &editor.error {
        lines.push(Line::from(Span::styled(
            error,
            Style::default().fg(Color::Red),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title(" Configure input mapping "))
            .alignment(Alignment::Left),
        area,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let age = app
        .last_update
        .map(|updated| format!("{} ms ago", updated.elapsed().as_millis()))
        .unwrap_or_else(|| "waiting for first report".to_owned());
    let (connection_label, connection_color, identity) = app
        .identity
        .as_ref()
        .map(|identity| {
            (
                "CONNECTED",
                Color::Green,
                format!("{} {}", identity.manufacturer, identity.product),
            )
        })
        .unwrap_or((
            "INPUT UNAVAILABLE",
            Color::Yellow,
            "SAILI not found".to_owned(),
        ));
    let (
        format,
        confidence,
        reason,
        reports,
        decoded,
        malformed,
        coalesced,
        reconnects,
        mux_losses,
    ) = app
        .reader_snapshot
        .as_ref()
        .map(|snapshot| {
            (
                snapshot
                    .stats
                    .selected_format
                    .map_or_else(|| "uncertain".to_owned(), |format| format.to_string()),
                snapshot.stats.format_confidence.label().to_owned(),
                snapshot.stats.selection_reason.clone(),
                snapshot.stats.reports_received,
                snapshot.stats.decoded_states,
                snapshot.stats.malformed_reports,
                snapshot.stats.coalesced_updates,
                snapshot.stats.reconnects,
                snapshot.stats.mux_loss_events,
            )
        })
        .unwrap_or_else(|| {
            (
                "--".to_owned(),
                "--".to_owned(),
                "--".to_owned(),
                0,
                0,
                0,
                0,
                0,
                0,
            )
        });
    let raw_age = app
        .reader_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.stats.last_raw_at)
        .map(|updated| format!("{} ms", updated.elapsed().as_millis()))
        .unwrap_or_else(|| "--".to_owned());
    let complete_age = app
        .reader_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.stats.last_complete_at)
        .map(|updated| format!("{} ms", updated.elapsed().as_millis()))
        .unwrap_or_else(|| "--".to_owned());
    let mux = app
        .reader_snapshot
        .as_ref()
        .map(|snapshot| {
            let status = snapshot.stats.decoder_status;
            let mux_problem = status
                .calibration_error
                .map_or_else(String::new, |error| format!("  •  {error}"));
            let loss_problem = status
                .loss_reason
                .map_or_else(String::new, |reason| format!("  •  {reason}"));
            format!(
                "mux {}  •  samples {}/{}  •  phase {}  •  phases {}/{}{}{}",
                status
                    .mux_state
                    .map_or_else(|| "n/a".to_owned(), |state| state.label().to_owned()),
                status.calibration_samples,
                status.calibration_target,
                status
                    .current_phase
                    .map_or_else(|| "--".to_owned(), |phase| phase.to_string()),
                u8::from(status.mux_seen[0]),
                u8::from(status.mux_seen[1]),
                mux_problem,
                loss_problem
            )
        })
        .unwrap_or_else(|| "mux --  •  samples --/--  •  phase --  •  phases --/--".to_owned());
    let first_line = Line::from(vec![
        Span::styled(
            connection_label,
            Style::default()
                .fg(connection_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  {}  •  format {} ({})  •  raw {} / complete {}  •  bad {}  •  mux-loss {}  •  coalesced {}  •  reconnects {}  •  {}",
            identity,
            format,
            confidence,
            reports,
            decoded,
            malformed,
            mux_losses,
            coalesced,
            reconnects,
            age
        )),
    ]);
    let second_line = Line::from(format!(
        "raw age {raw_age}  •  complete age {complete_age}  •  {mux}  •  {reason}"
    ));

    frame.render_widget(
        Paragraph::new(vec![first_line, second_line])
            .block(Block::bordered().title(" SAILI Controller ")),
        area,
    );
}

fn render_channels(frame: &mut Frame, area: Rect, state: Option<DecodedState>) {
    let block = Block::bordered().title(" Analogue inputs ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([Constraint::Length(1); CHANNEL_COUNT]).split(inner);
    for (index, row) in rows.iter().enumerate() {
        let value = state.and_then(|current| current.channel(index));
        let (ratio, label) = value.map_or(
            (0.0, format!("CH{}  --  unavailable", index + 1)),
            |value| {
                let percentage = u16::from(value) * 100 / 255;
                (
                    f64::from(value) / 255.0,
                    format!("CH{}  {value:3}  {percentage:3}%", index + 1),
                )
            },
        );
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Cyan))
            .ratio(ratio)
            .label(label)
            .use_unicode(true);
        frame.render_widget(gauge, *row);
    }
}

fn render_rc_input(frame: &mut Frame, area: Rect, app: &App) {
    let (mode, mode_color) = match app.output_mode {
        OutputMode::SafeHold => ("SAFE HOLD", Color::Yellow),
        OutputMode::Live => ("LIVE", Color::Green),
    };
    let block = Block::bordered().title(Line::from(vec![
        Span::raw(" RC mapping • "),
        Span::styled(
            mode,
            Style::default().fg(mode_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ]));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([Constraint::Length(1); 5]).split(inner);
    let controls = [
        ("ROLL", app.mapped_input.roll()),
        ("PITCH", app.mapped_input.pitch()),
        ("THROTTLE", app.mapped_input.throttle()),
        ("YAW", app.mapped_input.yaw()),
    ];

    for (row, (name, value)) in rows.iter().take(4).zip(controls) {
        let ratio = f64::from(value.saturating_sub(RC_MIN_US)) / f64::from(RC_MAX_US - RC_MIN_US);
        frame.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(Color::Blue))
                .ratio(ratio)
                .label(format!("{name:<8} {value:4} us"))
                .use_unicode(true),
            *row,
        );
    }

    let (arm_label, arm_color) = if app.output_mode == OutputMode::Live && app.mapped_input.armed()
    {
        ("ON", Color::Red)
    } else {
        ("OFF", Color::Green)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("ARM OUTPUT "),
            Span::styled(
                arm_label,
                Style::default().fg(arm_color).add_modifier(Modifier::BOLD),
            ),
            Span::raw(if app.arm.has_source() {
                "  •  CH5 / configured arm source"
            } else {
                "  •  CH5 forced low / arm unmapped"
            }),
        ]))
        .alignment(Alignment::Center),
        rows[4],
    );
}

fn render_backend(frame: &mut Frame, area: Rect, app: &App) {
    let status = match &app.backend_state {
        BackendState::Starting => Line::from(Span::styled("STARTING", Color::Yellow)),
        BackendState::Connecting { address } => Line::from(vec![
            Span::styled("CONNECTING", Color::Yellow),
            Span::raw(format!("  {address}")),
        ]),
        BackendState::Connected { server } => Line::from(vec![
            Span::styled(
                "CONNECTED",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {}  {}",
                server.name,
                if server.version.is_empty() {
                    server.mac_address.as_str()
                } else {
                    server.version.as_str()
                }
            )),
        ]),
        BackendState::Failed { message } => Line::from(vec![
            Span::styled("RETRYING", Color::Red),
            Span::raw(format!("  {message}")),
        ]),
    };
    let round_trip = app
        .last_round_trip
        .map(|duration| format!("{} ms", duration.as_millis()))
        .unwrap_or_else(|| "--".to_owned());
    let output = if app.safe_override { "safe" } else { "live" };
    let statistics = Line::from(format!(
        "sent {}  •  acknowledged {}  •  RTT {}  •  wire output {}",
        app.commands_sent, app.commands_acknowledged, round_trip, output
    ));

    frame.render_widget(
        Paragraph::new(vec![status, statistics])
            .block(Block::bordered().title(" ESPHome CRSF bridge ")),
        area,
    );
}

fn render_telemetry(frame: &mut Frame, area: Rect, telemetry: &FlightControllerTelemetry) {
    let age = telemetry
        .last_update
        .map(|updated| format!("{} ms", updated.elapsed().as_millis()))
        .unwrap_or_else(|| "--".to_owned());
    let last_type = telemetry
        .last_frame
        .as_ref()
        .map(|current| format!("0x{:02X}", current.frame_type()))
        .unwrap_or_else(|| "--".to_owned());
    let last_kind = telemetry.last_kind.unwrap_or("waiting");
    let status = Line::from(format!(
        "frames {}  •  rejected {}  •  age {}  •  last {} {last_type}",
        telemetry.frames_received, telemetry.frames_rejected, age, last_kind
    ));

    let battery = telemetry
        .battery
        .as_ref()
        .map(|current| {
            format!(
                "{:.1} V  {:.1} A  {} mAh  {}%",
                current.voltage_v,
                current.current_a,
                current.capacity_mah,
                current.remaining_percent
            )
        })
        .unwrap_or_else(|| "--".to_owned());
    let mode = telemetry.flight_mode.as_deref().unwrap_or("--");
    let vario = telemetry
        .vario_ms
        .map(|current| format!("{current:+.2} m/s"))
        .unwrap_or_else(|| "--".to_owned());
    let power = Line::from(format!(
        "battery {battery}  •  mode {mode}  •  vario {vario}"
    ));

    let attitude = telemetry
        .attitude
        .as_ref()
        .map(|current| {
            format!(
                "pitch {:+.1}°  roll {:+.1}°  yaw {:+.1}°",
                current.pitch_radians.to_degrees(),
                current.roll_radians.to_degrees(),
                current.yaw_radians.to_degrees()
            )
        })
        .unwrap_or_else(|| "pitch --  roll --  yaw --".to_owned());

    let gps = telemetry
        .gps
        .as_ref()
        .map(|current| {
            format!(
                "{:.6}, {:.6}  {} sat  {:.1} km/h  {:.1}°  {} m",
                current.latitude_degrees,
                current.longitude_degrees,
                current.satellites,
                current.ground_speed_kmh,
                current.heading_degrees,
                current.altitude_metres
            )
        })
        .unwrap_or_else(|| "--".to_owned());

    let barometer = telemetry
        .barometer
        .as_ref()
        .map(|current| format!("{} Pa  {:.2}°C", current.pressure_pa, current.temperature_c))
        .unwrap_or_else(|| "--".to_owned());
    let barometric_altitude = telemetry
        .barometric_altitude_metres
        .map(|current| format!("{current:.1} m"))
        .unwrap_or_else(|| "--".to_owned());
    let magnetometer = telemetry
        .magnetometer
        .as_ref()
        .map(|current| format!("x {}  y {}  z {}", current.x, current.y, current.z))
        .unwrap_or_else(|| "--".to_owned());
    let range = telemetry
        .range
        .as_ref()
        .map(|current| {
            format!(
                "F {}  B {}  L {}  R {}",
                format_range(current.front_metres),
                format_range(current.back_metres),
                format_range(current.left_metres),
                format_range(current.right_metres)
            )
        })
        .unwrap_or_else(|| "F --  B --  L --  R --".to_owned());
    let environment = telemetry.last_error.as_ref().map_or_else(
        || {
            format!(
                "range {range}  •  baro {barometric_altitude} {barometer}  •  mag {magnetometer}"
            )
        },
        |error| format!("error {error}"),
    );

    let raw_width = usize::from(area.width.saturating_sub(9));
    let raw = telemetry
        .last_frame
        .as_ref()
        .map(CrsfFrame::raw_hex)
        .map(|value| truncate_ascii(value, raw_width))
        .unwrap_or_else(|| "--".to_owned());

    frame.render_widget(
        Paragraph::new(vec![
            status,
            power,
            Line::from(format!("attitude {attitude}")),
            Line::from(format!("GPS {gps}")),
            Line::from(environment),
            Line::from(format!("raw {raw}")),
        ])
        .block(Block::bordered().title(" FC CRSF telemetry ")),
        area,
    );
}

fn format_range(range_metres: Option<f32>) -> String {
    range_metres
        .map(|current| format!("{current:.2}m"))
        .unwrap_or_else(|| "--".to_owned())
}

fn truncate_ascii(mut value: String, maximum: usize) -> String {
    if value.len() <= maximum {
        return value;
    }
    if maximum <= 3 {
        return ".".repeat(maximum);
    }
    value.truncate(maximum - 3);
    value.push_str("...");
    value
}

fn render_raw_packet(frame: &mut Frame, area: Rect, raw_report: Option<saili::RawReport>) {
    let raw = raw_report
        .map(|current| {
            current
                .bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|| "-- -- -- -- -- -- -- --".to_owned());

    frame.render_widget(
        Paragraph::new(raw)
            .alignment(Alignment::Center)
            .block(Block::bordered().title(" Raw HID report ")),
        area,
    );
}

fn render_help(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(
        Paragraph::new(format!(
            "{}  •  p calibrate mux  •  Enter confirm step  •  m mapping  •  l live/safe  •  q / Esc / Ctrl-C quit",
            app.notice
        ))
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

#[derive(Debug, Parser)]
#[command(version, about = "Stream a SAILI controller to an ESPHome CRSF bridge")]
struct Arguments {
    #[arg(
        long,
        env = "SAILI_ESPHOME_ADDRESS",
        default_value = "madflight-rc-bridge.local:6053"
    )]
    esphome_address: String,

    #[arg(long, env = "SAILI_ESPHOME_KEY", hide_env_values = true)]
    esphome_key: String,

    #[arg(long, default_value_t = DEFAULT_TRANSMIT_RATE_HZ)]
    transmit_rate_hz: u16,

    #[arg(long, default_value_t = 1)]
    roll_channel: usize,

    #[arg(long, default_value_t = 2)]
    pitch_channel: usize,

    #[arg(long, default_value_t = 3)]
    throttle_channel: usize,

    #[arg(long, default_value_t = 4)]
    yaw_channel: usize,

    #[arg(long)]
    invert_roll: bool,

    #[arg(long)]
    invert_pitch: bool,

    #[arg(long)]
    invert_throttle: bool,

    #[arg(long)]
    invert_yaw: bool,

    #[arg(long, default_value = "auto")]
    report_format: ReportFormat,

    #[arg(long)]
    swap_mux_channels: bool,

    #[arg(long)]
    arm_threshold: Option<u8>,

    #[arg(long)]
    arm_channel: Option<usize>,

    #[arg(long)]
    invert_arm: bool,
}

struct RuntimeConfig {
    backend: BackendConfig,
    mapping: RcMapping,
    reader: ReaderConfig,
    arm: ArmConfig,
}

impl TryFrom<Arguments> for RuntimeConfig {
    type Error = ConfigError;

    fn try_from(arguments: Arguments) -> Result<Self, Self::Error> {
        if !(1..=MAX_TRANSMIT_RATE_HZ).contains(&arguments.transmit_rate_hz) {
            return Err(ConfigError::TransmitRate {
                value: arguments.transmit_rate_hz,
                maximum: MAX_TRANSMIT_RATE_HZ,
            });
        }

        let mapping = RcMapping::new(
            [
                arguments.roll_channel,
                arguments.pitch_channel,
                arguments.throttle_channel,
                arguments.yaw_channel,
            ],
            [
                arguments.invert_roll,
                arguments.invert_pitch,
                arguments.invert_throttle,
                arguments.invert_yaw,
            ],
        )?;

        let arm_channel = match arguments.arm_channel {
            Some(channel) if (1..=CHANNEL_COUNT).contains(&channel) => Some(channel - 1),
            Some(channel) => return Err(ConfigError::ArmChannel { channel }),
            None => None,
        };
        let arm = ArmConfig {
            channel: arm_channel,
            threshold: arguments.arm_threshold.unwrap_or(127),
            hysteresis: 4,
            inverted: arguments.invert_arm,
        };

        Ok(Self {
            backend: BackendConfig {
                address: arguments.esphome_address,
                encryption_key: arguments.esphome_key,
                transmit_interval: Duration::from_secs_f64(
                    1.0 / f64::from(arguments.transmit_rate_hz),
                ),
            },
            mapping,
            reader: ReaderConfig {
                report_format: arguments.report_format,
                swap_mux_channels: arguments.swap_mux_channels,
            },
            arm,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ConfigError {
    #[error("transmit rate {value} Hz is invalid; use 1-{maximum} Hz")]
    TransmitRate { value: u16, maximum: u16 },

    #[error(transparent)]
    Mapping(#[from] MappingError),

    #[error("arm channel {channel} is invalid; use 1-{CHANNEL_COUNT}")]
    ArmChannel { channel: usize },
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Device(#[from] SailiError),

    #[error(transparent)]
    Reader(#[from] ReaderStartError),

    #[error(transparent)]
    Backend(#[from] BackendError),

    #[error("terminal operation failed")]
    Terminal(#[source] io::Error),

    #[error("terminal input failed")]
    Input(#[source] io::Error),
}
