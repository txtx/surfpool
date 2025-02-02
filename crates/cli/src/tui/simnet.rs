use std::{collections::VecDeque, error::Error, io, sync::mpsc::Receiver, time::Duration};

use chrono::{DateTime, Local};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    style::palette::{self, tailwind},
    widgets::*,
};
use surfpool_core::{
    simnet::SimnetEvent,
    solana_sdk::{clock::Clock, epoch_info::EpochInfo},
};

const HELP_TEXT: &str = "(Esc) quit | (↑) move up | (↓) move down";
const TXTX_LINK: &str = "https://txtx.run";

const ITEM_HEIGHT: usize = 1;

struct ColorTheme {
    background: Color,
    accent: Color,
    primary: Color,
    secondary: Color,
    white: Color,
    gray: Color,
    error: Color,
    warning: Color,
    info: Color,
    success: Color,
}

impl ColorTheme {
    fn new(color: &tailwind::Palette) -> Self {
        Self {
            background: tailwind::SLATE.c950,
            accent: color.c400,
            primary: color.c500,
            secondary: color.c700,
            white: tailwind::SLATE.c200,
            gray: tailwind::SLATE.c500,
            error: tailwind::RED.c400,
            warning: tailwind::YELLOW.c500,
            info: tailwind::BLUE.c500,
            success: tailwind::GREEN.c500,
        }
    }
}

enum EventType {
    Debug,
    Info,
    Success,
    Failure,
    Warning,
}

struct App {
    state: TableState,
    scroll_state: ScrollbarState,
    colors: ColorTheme,
    simnet_events_rx: Receiver<SimnetEvent>,
    clock: Clock,
    epoch_info: EpochInfo,
    events: VecDeque<(EventType, DateTime<Local>, String)>,
    include_debug_logs: bool,
}

impl App {
    fn new(simnet_events_rx: Receiver<SimnetEvent>, include_debug_logs: bool) -> App {
        App {
            state: TableState::default().with_selected(0),
            scroll_state: ScrollbarState::new(5 * ITEM_HEIGHT),
            colors: ColorTheme::new(&palette::tailwind::EMERALD),
            simnet_events_rx,
            clock: Clock::default(),
            epoch_info: EpochInfo {
                epoch: 0,
                slot_index: 0,
                slots_in_epoch: 0,
                absolute_slot: 0,
                block_height: 0,
                transaction_count: None,
            },
            events: VecDeque::new(),
            include_debug_logs,
        }
    }

    pub fn slot(&self) -> usize {
        self.clock.slot.try_into().unwrap()
    }

    pub fn epoch_progress(&self) -> u16 {
        let current = self.slot() as u64;
        let expected = self.epoch_info.slots_in_epoch;
        if expected == 0 {
            return 100;
        }
        ((current.min(expected) as f64 / expected as f64) * 100.0) as u16
    }

    pub fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => i,
            None => 0,
        };
        self.state.select(Some(i));
        self.scroll_state = self.scroll_state.position(i * ITEM_HEIGHT);
    }

    pub fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => i,
            None => 0,
        };
        self.state.select(Some(i));
        self.scroll_state = self.scroll_state.position(i * ITEM_HEIGHT);
    }

    pub fn set_colors(&mut self) {
        self.colors = ColorTheme::new(&tailwind::EMERALD)
    }
}

pub fn start_app(
    simnet_events_rx: Receiver<SimnetEvent>,
    include_debug_logs: bool,
) -> Result<(), Box<dyn Error>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // create app and run it
    let app = App::new(simnet_events_rx, include_debug_logs);
    let res = run_app(&mut terminal, app);

    // restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{err:?}");
    }

    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> io::Result<()> {
    loop {
        while let Ok(event) = app.simnet_events_rx.try_recv() {
            match event {
                SimnetEvent::AccountUpdate(dt, account) => {
                    app.events.push_front((
                        EventType::Success,
                        dt,
                        format!("Account {} retrieved from Mainnet", account),
                    ));
                }
                SimnetEvent::EpochInfoUpdate(epoch_info) => {
                    app.epoch_info = epoch_info;
                    app.events.push_front((
                        EventType::Success,
                        Local::now(),
                        format!(
                            "Connection established at Slot {} / Epoch {}.",
                            app.epoch_info.epoch, app.epoch_info.slot_index
                        ),
                    ));
                }
                SimnetEvent::ClockUpdate(clock) => {
                    app.clock = clock;
                    if app.include_debug_logs {
                        app.events
                            .push_front((EventType::Debug, Local::now(), "Clock updated".into()));
                    }
                }
                SimnetEvent::ErrorLog(dt, log) => {
                    app.events.push_front((EventType::Failure, dt, log));
                }
                SimnetEvent::InfoLog(dt, log) => {
                    app.events.push_front((EventType::Info, dt, log));
                }
                SimnetEvent::DebugLog(dt, log) => {
                    if app.include_debug_logs {
                        app.events.push_front((EventType::Debug, dt, log));
                    }
                }
                SimnetEvent::WarnLog(dt, log) => {
                    app.events.push_front((EventType::Warning, dt, log));
                }
                SimnetEvent::TransactionReceived(dt, _transaction) => {
                    app.events.push_front((
                        EventType::Success,
                        dt,
                        format!("Transaction received"),
                    ));
                }
                SimnetEvent::BlockHashExpired => {}
            }
        }

        terminal.draw(|f| ui(f, &mut app))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    use KeyCode::*;
                    match key.code {
                        Char('q') | Esc => return Ok(()),
                        Char('j') | Down => app.next(),
                        Char('k') | Up => app.previous(),
                        _ => {}
                    }
                }
            }
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let rects = Layout::vertical([
        Constraint::Length(7),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .split(f.area());
    app.set_colors();

    let default_style = Style::new()
        .fg(app.colors.secondary)
        .bg(app.colors.background);
    let chrome = Block::default()
        .style(default_style.clone())
        .borders(Borders::ALL)
        .border_style(default_style)
        .border_type(BorderType::Plain);
    f.render_widget(chrome, f.area());

    render_epoch(f, app, rects[0].inner(Margin::new(1, 1)));
    render_events(f, app, rects[1].inner(Margin::new(2, 0)));
    render_scrollbar(f, app, rects[1].inner(Margin::new(0, 0)));
    render_footer(f, app, rects[2].inner(Margin::new(2, 1)));
}

fn title_block(title: &str, alignment: Alignment) -> Block {
    let title = Line::from(title).alignment(alignment);
    Block::new().borders(Borders::NONE).title(title)
}

fn render_epoch(f: &mut Frame, app: &mut App, area: Rect) {
    let columns = Layout::horizontal([
        Constraint::Length(7),  // Slots Title
        Constraint::Min(30),    // Slots
        Constraint::Length(1),  // Leader Details
        Constraint::Length(50), // Leader Details
    ])
    .split(area);

    let titles = Layout::vertical([
        Constraint::Length(3), // Slots
        Constraint::Length(1), // Divider
        Constraint::Length(1), // Epoch
    ])
    .split(columns[0]);

    let widgets = Layout::vertical([
        Constraint::Length(3), // Slots
        Constraint::Length(1), // Divider
        Constraint::Length(1), // Epoch
    ])
    .split(columns[1]);

    let title = title_block("Slots", Alignment::Center).style(app.colors.secondary);
    f.render_widget(title, titles[0].inner(Margin::new(1, 1)));

    render_slots(f, app, widgets[0].inner(Margin::new(1, 0)));

    let title = title_block("Epoch", Alignment::Center);
    f.render_widget(title, titles[2].inner(Margin::new(1, 0)));

    let epoch_progress = Gauge::default()
        .gauge_style(app.colors.primary)
        .bg(app.colors.gray)
        .percent(app.epoch_progress());
    f.render_widget(epoch_progress, widgets[2].inner(Margin::new(1, 0)));

    let default_style = Style::new().fg(app.colors.gray);

    let separator = Block::default()
        .style(default_style.clone())
        .borders(Borders::LEFT)
        .border_style(default_style)
        .border_type(BorderType::Plain);
    f.render_widget(separator, columns[3]);
}

fn render_slots(f: &mut Frame, app: &mut App, area: Rect) {
    let line_len = area.width as usize;
    let total_chars = line_len * 3;
    let cursor = app.slot() % total_chars;
    let sequence: Vec<char> = (0..total_chars)
        .map(|i| if i < cursor { '▮' } else { '▯' })
        .collect();

    let text: String = sequence
        .chunks(line_len)
        .map(|line| line.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");

    let title = Paragraph::new(text);
    f.render_widget(title.style(app.colors.accent), area);
}

fn render_events(f: &mut Frame, app: &mut App, area: Rect) {
    let rects = Layout::vertical([
        Constraint::Length(2), // Title
        Constraint::Min(1),    // Logs
    ])
    .split(area);

    let symbol = ['⠈', '⠘', '⠸', '⠼', '⠾', '⠿', '⠷', '⠧', '⠇', '⠃', '⠁', '⠀'];
    let cursor = symbol[app.slot() % 12];
    let title = Block::new()
        .padding(Padding::symmetric(4, 4))
        .borders(Borders::NONE)
        .title(Line::from(format!("Activity {}", cursor)));
    f.render_widget(title, rects[0]);

    let rows = app.events.iter().map(|(event_type, dt, log)| {
        let color = match event_type {
            EventType::Failure => app.colors.error,
            EventType::Info => app.colors.info,
            EventType::Success => app.colors.success,
            EventType::Warning => app.colors.warning,
            EventType::Debug => app.colors.gray,
        };
        let row = vec![
            Cell::new("⏐").style(color),
            Cell::new(dt.format("%H:%M:%S.%3f").to_string()).style(app.colors.gray),
            Cell::new(log.to_string()),
        ];
        Row::new(row)
            .style(Style::new().fg(app.colors.white))
            .height(1)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(12),
            Constraint::Min(1),
        ],
    )
    .style(Style::new().fg(app.colors.white).bg(app.colors.background))
    .highlight_spacing(HighlightSpacing::Always);
    f.render_stateful_widget(table, rects[1], &mut app.state);
}

fn render_scrollbar(f: &mut Frame, app: &mut App, area: Rect) {
    f.render_stateful_widget(
        Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        area.inner(Margin {
            vertical: 1,
            horizontal: 1,
        }),
        &mut app.scroll_state,
    );
}

fn render_footer(f: &mut Frame, app: &mut App, area: Rect) {
    let rects = Layout::horizontal([
        Constraint::Min(30),    // Help
        Constraint::Length(20), // https://txtx.run
    ])
    .split(area);

    let help = title_block(HELP_TEXT, Alignment::Left).style(Style::new().fg(app.colors.gray));
    f.render_widget(help, rects[0]);

    let link = title_block(TXTX_LINK, Alignment::Right).style(Style::new().fg(app.colors.white));
    f.render_widget(link, rects[1]);
}
