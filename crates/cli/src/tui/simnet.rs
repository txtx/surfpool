use std::{collections::VecDeque, error::Error, io, sync::mpsc::Receiver, time::Duration};

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
use surfpool_core::{simnet::SimnetEvent, solana_sdk::clock::Clock};

const HELP_TEXT: &str = "(Esc) quit | (↑) move up | (↓) move down";
const TXTX_LINK: &str = "https://txtx.run";

const ITEM_HEIGHT: usize = 4;

struct TableColors {
    buffer_bg: Color,
    accent_color: Color,
    primary_color: Color,
    secondary_color: Color,
    white_color: Color,
    gray_color: Color,
}

impl TableColors {
    fn new(color: &tailwind::Palette) -> Self {
        Self {
            buffer_bg: tailwind::SLATE.c950,
            accent_color: color.c400,
            primary_color: color.c500,
            secondary_color: color.c900,
            white_color: tailwind::SLATE.c200,
            gray_color: tailwind::SLATE.c900,
        }
    }
}

enum EventType {
    Info,
    Success,
    Failure,
    Warning,
}

struct App {
    state: TableState,
    scroll_state: ScrollbarState,
    colors: TableColors,
    simnet_events_rx: Receiver<SimnetEvent>,
    clock: Clock,
    events: VecDeque<(EventType, String)>,
}

impl App {
    fn new(simnet_events_rx: Receiver<SimnetEvent>) -> App {
        App {
            state: TableState::default().with_selected(0),
            scroll_state: ScrollbarState::new(5 * ITEM_HEIGHT),
            colors: TableColors::new(&palette::tailwind::EMERALD),
            simnet_events_rx,
            clock: Clock::default(),
            events: VecDeque::new(),
        }
    }

    pub fn epoch(&self) -> usize {
        self.clock.epoch.try_into().unwrap()
    }

    pub fn slot(&self) -> usize {
        self.clock.slot.try_into().unwrap()
    }

    pub fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => 0,
            None => 0,
        };
        self.state.select(Some(i));
        self.scroll_state = self.scroll_state.position(i * ITEM_HEIGHT);
    }

    pub fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => 0,
            None => 0,
        };
        self.state.select(Some(i));
        self.scroll_state = self.scroll_state.position(i * ITEM_HEIGHT);
    }

    pub fn set_colors(&mut self) {
        self.colors = TableColors::new(&tailwind::EMERALD)
    }
}

pub fn start_app(simnet_events_rx: Receiver<SimnetEvent>) -> Result<(), Box<dyn Error>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // create app and run it
    let app = App::new(simnet_events_rx);
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
                SimnetEvent::AccountUpdate(account) => {
                    app.events.push_front((
                        EventType::Success,
                        format!("Account retrieved from Mainnet"),
                    ));
                }
                SimnetEvent::ClockUpdate(clock) => {
                    app.clock = clock;
                    app.events
                        .push_front((EventType::Failure, "Clock updated".into()));
                }
                SimnetEvent::ErroLog(log) => {
                    app.events.push_front((EventType::Failure, log));
                }
                SimnetEvent::InfoLog(log) => {
                    app.events.push_front((EventType::Info, log));
                }
                SimnetEvent::WarnLog(log) => {
                    app.events.push_front((EventType::Warning, log));
                }
                SimnetEvent::TransactionReceived(transaction) => {
                    app.events
                        .push_front((EventType::Success, format!("Transaction received")));
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
        Constraint::Length(5),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .split(f.area());
    app.set_colors();

    let default_style = Style::new()
        .fg(app.colors.secondary_color)
        .bg(app.colors.buffer_bg);
    let chrome = Block::default()
        .style(default_style.clone())
        .borders(Borders::ALL)
        .border_style(default_style)
        .border_type(BorderType::Plain);
    f.render_widget(chrome, f.area());

    render_epoch(f, app, rects[0].inner(Margin::new(1, 1)));
    render_events(f, app, rects[1].inner(Margin::new(2, 1)));
    render_scrollbar(f, app, rects[1]);
    render_footer(f, app, rects[2].inner(Margin::new(2, 1)));
}

fn title_block(title: &str, alignment: Alignment) -> Block {
    let title = Line::from(title).alignment(alignment);
    Block::new().borders(Borders::NONE).title(title)
}

fn render_epoch(f: &mut Frame, app: &mut App, area: Rect) {
    let rects = Layout::horizontal([
        Constraint::Length(6),  // Slots Title
        Constraint::Min(30),    // Slots
        Constraint::Length(7),  // Epoch Title
        Constraint::Length(20), // Progress bar
        Constraint::Length(40), // Leader Details
    ])
    .split(area);

    let title = title_block("Slots", Alignment::Center);
    f.render_widget(title, rects[0].inner(Margin::new(1, 1)));

    render_slots(f, app, rects[1].inner(Margin::new(1, 0)));

    let title = title_block("Epoch", Alignment::Center);
    f.render_widget(title, rects[2].inner(Margin::new(1, 1)));

    let epoch_progress = Gauge::default()
        .gauge_style(app.colors.secondary_color)
        .bg(app.colors.gray_color)
        .label(Span::raw(""))
        .percent(50);
    f.render_widget(epoch_progress, rects[3].inner(Margin::new(1, 1)));
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
    // title.style(app.colors.accent_color);
    f.render_widget(title.style(app.colors.accent_color), area);
}

fn render_events(f: &mut Frame, app: &mut App, area: Rect) {
    let symbol = ['⠈', '⠘', '⠸', '⠼', '⠾', '⠿', '⠷', '⠧', '⠇', '⠃', '⠁', '⠀'];
    let cursor = symbol[app.slot() % 12];
    let title = Block::new()
        .padding(Padding::symmetric(4, 4))
        .borders(Borders::NONE)
        .title(Line::from(format!("Activity {}", cursor)));

    f.render_widget(title, area);

    // let selected_style = Style::default()
    //     .add_modifier(Modifier::REVERSED)
    //     .fg(app.colors.selected_style_fg);

    let rows = app.events.iter().enumerate().map(|(i, (event_type, log))| {
        let row = vec![log.to_string()];
        Row::new(row)
            .style(Style::new().fg(app.colors.white_color))
            .height(1)
    });

    let bar = " █ ";
    let t = Table::new(
        rows,
        [
            // + 1 is for padding.
            Constraint::Length(64),
        ],
    )
    .style(
        Style::new()
            .fg(app.colors.white_color)
            .bg(app.colors.buffer_bg),
    )
    .highlight_symbol(Text::from(vec![
        "".into(),
        bar.into(),
        bar.into(),
        "".into(),
    ]))
    .block(
        Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::new().fg(app.colors.accent_color))
            .border_type(BorderType::Plain),
    )
    .highlight_spacing(HighlightSpacing::Always);
    f.render_stateful_widget(t, area, &mut app.state);
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

    let help =
        title_block(HELP_TEXT, Alignment::Left).style(Style::new().fg(app.colors.gray_color));
    f.render_widget(help, rects[0]);

    let link =
        title_block(TXTX_LINK, Alignment::Right).style(Style::new().fg(app.colors.white_color));
    f.render_widget(link, rects[1]);
}
