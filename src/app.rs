use std::io;
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Gauge, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use saili::{
    CHANNEL_COUNT, DeviceIdentity, DeviceState, MappingError, RC_MAX_US, RC_MIN_US, RcChannels,
    RcMapping, ReadStatus, SailiDevice, SailiError, ServerIdentity,
};
use thiserror::Error;

use crate::backend::{Backend, BackendConfig, BackendError, BackendEvent, OutputMode};

const DEVICE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(40);
const LIVE_INPUT_MAX_AGE: Duration = Duration::from_millis(150);
const SAFE_THROTTLE_MAX_US: u16 = 1050;
const DEFAULT_TRANSMIT_RATE_HZ: u16 = 20;
const MAX_TRANSMIT_RATE_HZ: u16 = 50;

pub fn run() -> Result<(), AppError> {
    let arguments = Arguments::parse();
    let config = RuntimeConfig::try_from(arguments)?;
    let device = SailiDevice::connect()?;
    let mut app = App::new(device.identity().clone(), config.mapping);
    let backend = Backend::start(config.backend)?;
    let mut terminal = match ratatui::try_init() {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = backend.shutdown();
            return Err(AppError::Terminal(error));
        }
    };

    let run_result = run_loop(&mut terminal, &device, &backend, &mut app);
    let shutdown_result = backend.shutdown();
    ratatui::restore();

    run_result?;
    shutdown_result?;
    Ok(())
}

fn run_loop(
    terminal: &mut DefaultTerminal,
    device: &SailiDevice,
    backend: &Backend,
    app: &mut App,
) -> Result<(), AppError> {
    loop {
        if let ReadStatus::State(state) = device.read_state(DEVICE_POLL_INTERVAL)? {
            app.update_input(state);
            backend.set_output(app.output_mode, app.mapped_input);
        }

        for backend_event in backend.drain_events() {
            app.update_backend(backend_event);
        }

        terminal
            .draw(|frame| render(frame, app))
            .map_err(AppError::Terminal)?;

        if event::poll(INPUT_POLL_INTERVAL).map_err(AppError::Input)? {
            match handle_event(event::read().map_err(AppError::Input)?, app) {
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

fn handle_event(event: Event, app: &mut App) -> AppCommand {
    let Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        ..
    }) = event
    else {
        return AppCommand::Continue;
    };

    if matches!(code, KeyCode::Esc | KeyCode::Char('q'))
        || (code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL))
    {
        return AppCommand::Quit;
    }

    if code == KeyCode::Char('l') {
        app.toggle_live();
    }

    AppCommand::Continue
}

enum AppCommand {
    Continue,
    Quit,
}

struct App {
    identity: DeviceIdentity,
    mapping: RcMapping,
    state: Option<DeviceState>,
    mapped_input: RcChannels,
    output_mode: OutputMode,
    reports_received: u64,
    last_update: Option<Instant>,
    backend_state: BackendState,
    commands_sent: u64,
    commands_acknowledged: u64,
    last_round_trip: Option<Duration>,
    safe_override: bool,
    notice: String,
}

impl App {
    fn new(identity: DeviceIdentity, mapping: RcMapping) -> Self {
        Self {
            identity,
            mapping,
            state: None,
            mapped_input: RcChannels::safe(),
            output_mode: OutputMode::SafeHold,
            reports_received: 0,
            last_update: None,
            backend_state: BackendState::Starting,
            commands_sent: 0,
            commands_acknowledged: 0,
            last_round_trip: None,
            safe_override: true,
            notice: "Safe hold active; press l with throttle low and arm off".to_owned(),
        }
    }

    fn update_input(&mut self, state: DeviceState) {
        self.mapped_input = self.mapping.map(state);
        self.state = Some(state);
        self.reports_received = self.reports_received.saturating_add(1);
        self.last_update = Some(Instant::now());
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

        let Some(last_update) = self.last_update else {
            self.notice = "Cannot go live before the first controller report".to_owned();
            return;
        };
        if last_update.elapsed() > LIVE_INPUT_MAX_AGE {
            self.notice = "Cannot go live while controller input is stale".to_owned();
            return;
        }
        if self.mapped_input.throttle() > SAFE_THROTTLE_MAX_US {
            self.notice = format!(
                "Cannot go live: throttle is {} us (must be <= {SAFE_THROTTLE_MAX_US})",
                self.mapped_input.throttle()
            );
            return;
        }
        if self.mapped_input.armed() {
            self.notice = "Cannot go live while the arm switch is on".to_owned();
            return;
        }

        self.output_mode = OutputMode::Live;
        self.notice = "Live controller forwarding enabled".to_owned();
    }
}

enum BackendState {
    Starting,
    Connecting { address: String },
    Connected { server: ServerIdentity },
    Failed { message: String },
}

fn render(frame: &mut Frame, app: &App) {
    let [
        status_area,
        channels_area,
        rc_area,
        backend_area,
        raw_area,
        help_area,
    ] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length((CHANNEL_COUNT + 2) as u16),
        Constraint::Length(7),
        Constraint::Length(4),
        Constraint::Length(3),
        Constraint::Min(1),
    ])
    .areas(frame.area());

    render_status(frame, status_area, app);
    render_channels(frame, channels_area, app.state);
    render_rc_input(frame, rc_area, app);
    render_backend(frame, backend_area, app);
    render_raw_packet(frame, raw_area, app.state);
    render_help(frame, help_area, app);
}

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let age = app
        .last_update
        .map(|updated| format!("{} ms ago", updated.elapsed().as_millis()))
        .unwrap_or_else(|| "waiting for first report".to_owned());
    let text = Line::from(vec![
        Span::styled(
            "CONNECTED",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  {} {}  •  reports {}  •  {}",
            app.identity.manufacturer, app.identity.product, app.reports_received, age
        )),
    ]);

    frame.render_widget(
        Paragraph::new(text).block(Block::bordered().title(" SAILI Controller ")),
        area,
    );
}

fn render_channels(frame: &mut Frame, area: Rect, state: Option<DeviceState>) {
    let block = Block::bordered().title(" Analogue inputs ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([Constraint::Length(1); CHANNEL_COUNT]).split(inner);
    let channels = state
        .map(|current| *current.channels())
        .unwrap_or([0; CHANNEL_COUNT]);

    for (index, (row, value)) in rows.iter().zip(channels).enumerate() {
        let percentage = u16::from(value) * 100 / 255;
        let label = format!("CH{}  {value:3}  {percentage:3}%", index + 1);
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Cyan))
            .ratio(f64::from(value) / 255.0)
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

    let (arm_label, arm_color) = if app.mapped_input.armed() {
        ("ON", Color::Red)
    } else {
        ("OFF", Color::Green)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("ARM INPUT "),
            Span::styled(
                arm_label,
                Style::default().fg(arm_color).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  •  CH5 / aux1"),
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

fn render_raw_packet(frame: &mut Frame, area: Rect, state: Option<DeviceState>) {
    let raw = state
        .map(|current| {
            current
                .raw()
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
            "{}  •  l live/safe  •  q / Esc / Ctrl-C quit",
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
}

struct RuntimeConfig {
    backend: BackendConfig,
    mapping: RcMapping,
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

        Ok(Self {
            backend: BackendConfig {
                address: arguments.esphome_address,
                encryption_key: arguments.esphome_key,
                transmit_interval: Duration::from_secs_f64(
                    1.0 / f64::from(arguments.transmit_rate_hz),
                ),
            },
            mapping,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ConfigError {
    #[error("transmit rate {value} Hz is invalid; use 1-{maximum} Hz")]
    TransmitRate { value: u16, maximum: u16 },

    #[error(transparent)]
    Mapping(#[from] MappingError),
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Device(#[from] SailiError),

    #[error(transparent)]
    Backend(#[from] BackendError),

    #[error("terminal operation failed")]
    Terminal(#[source] io::Error),

    #[error("terminal input failed")]
    Input(#[source] io::Error),
}
