use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Gauge, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use saili::{CHANNEL_COUNT, DeviceIdentity, DeviceState, ReadStatus, SailiDevice, SailiError};
use thiserror::Error;

const DEVICE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(40);

pub fn run() -> Result<(), AppError> {
    let device = SailiDevice::connect()?;
    let mut app = App::new(device.identity().clone());
    let mut terminal = ratatui::try_init().map_err(AppError::Terminal)?;

    let result = run_loop(&mut terminal, &device, &mut app);
    ratatui::restore();
    result
}

fn run_loop(
    terminal: &mut DefaultTerminal,
    device: &SailiDevice,
    app: &mut App,
) -> Result<(), AppError> {
    loop {
        if let ReadStatus::State(state) = device.read_state(DEVICE_POLL_INTERVAL)? {
            app.update(state);
        }

        terminal
            .draw(|frame| render(frame, app))
            .map_err(AppError::Terminal)?;

        if event::poll(INPUT_POLL_INTERVAL).map_err(AppError::Input)?
            && should_quit(event::read().map_err(AppError::Input)?)
        {
            return Ok(());
        }
    }
}

fn should_quit(event: Event) -> bool {
    let Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        ..
    }) = event
    else {
        return false;
    };

    matches!(code, KeyCode::Esc | KeyCode::Char('q'))
        || (code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL))
}

struct App {
    identity: DeviceIdentity,
    state: Option<DeviceState>,
    reports_received: u64,
    last_update: Option<Instant>,
}

impl App {
    fn new(identity: DeviceIdentity) -> Self {
        Self {
            identity,
            state: None,
            reports_received: 0,
            last_update: None,
        }
    }

    fn update(&mut self, state: DeviceState) {
        self.state = Some(state);
        self.reports_received = self.reports_received.saturating_add(1);
        self.last_update = Some(Instant::now());
    }
}

fn render(frame: &mut Frame, app: &App) {
    let [status_area, channels_area, switch_area, raw_area, help_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length((CHANNEL_COUNT + 2) as u16),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(1),
    ])
    .areas(frame.area());

    render_status(frame, status_area, app);
    render_channels(frame, channels_area, app.state);
    render_switch(frame, switch_area, app.state);
    render_raw_packet(frame, raw_area, app.state);
    render_help(frame, help_area);
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
    let block = Block::bordered().title(" Channels ");
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

fn render_switch(frame: &mut Frame, area: Rect, state: Option<DeviceState>) {
    let enabled = state.is_some_and(|current| current.digital_switch());
    let (label, color) = if enabled {
        ("ON", Color::Green)
    } else {
        ("OFF", Color::DarkGray)
    };

    frame.render_widget(
        Paragraph::new(Span::styled(
            label,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center)
        .block(Block::bordered().title(" Digital switch ")),
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

fn render_help(frame: &mut Frame, area: Rect) {
    frame.render_widget(
        Paragraph::new("q / Esc / Ctrl-C  quit")
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Device(#[from] SailiError),

    #[error("terminal operation failed")]
    Terminal(#[source] io::Error),

    #[error("terminal input failed")]
    Input(#[source] io::Error),
}
