use chrono::{DateTime, Local};
use crossbeam::channel::{unbounded, Select, Sender};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    style::palette::{self, tailwind},
    widgets::*,
};
use solana_clock::Clock;
use solana_commitment_config::CommitmentConfig;
use solana_epoch_info::EpochInfo;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::Transaction;
use std::{collections::VecDeque, error::Error, io, time::Duration};
use surfpool_core::{solana_rpc_client::rpc_client::RpcClient, surfnet::SLOTS_PER_EPOCH};
use surfpool_types::{BlockProductionMode, ClockCommand, SimnetCommand, SimnetEvent};
use txtx_core::kit::types::frontend::BlockEvent;
use txtx_core::kit::{channel::Receiver, types::frontend::ProgressBarStatusColor};

const HELP_TEXT: &str = "(Esc) quit | (↑) move up | (↓) move down";
const SURFPOOL_LINK: &str = "Need help? https://docs.surfpool.run/tui";

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
            secondary: color.c300,
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
    simnet_commands_tx: Sender<SimnetCommand>,
    clock: Clock,
    epoch_info: EpochInfo,
    successful_transactions: u32,
    events: VecDeque<(EventType, DateTime<Local>, String)>,
    include_debug_logs: bool,
    deploy_progress_rx: Vec<Receiver<BlockEvent>>,
    status_bar_message: Option<String>,
    remote_rpc_url: String,
    local_rpc_url: String,
    breaker: Option<Keypair>,
    paused: bool,
}

impl App {
    fn new(
        simnet_events_rx: Receiver<SimnetEvent>,
        simnet_commands_tx: Sender<SimnetCommand>,
        include_debug_logs: bool,
        deploy_progress_rx: Vec<Receiver<BlockEvent>>,
        remote_rpc_url: &str,
        local_rpc_url: &str,
        breaker: Option<Keypair>,
    ) -> App {
        let palette = if remote_rpc_url.contains("helius") {
            palette::tailwind::RED
        } else {
            palette::tailwind::EMERALD
        };
        App {
            state: TableState::default().with_selected(0),
            scroll_state: ScrollbarState::new(5 * ITEM_HEIGHT),
            colors: ColorTheme::new(&palette),
            simnet_events_rx,
            simnet_commands_tx,
            clock: Clock::default(),
            epoch_info: EpochInfo {
                epoch: 0,
                slot_index: 0,
                slots_in_epoch: SLOTS_PER_EPOCH,
                absolute_slot: 0,
                block_height: 0,
                transaction_count: None,
            },
            successful_transactions: 0,
            events: VecDeque::new(),
            include_debug_logs,
            deploy_progress_rx,
            status_bar_message: None,
            remote_rpc_url: remote_rpc_url.to_string(),
            local_rpc_url: format!("http://{}", local_rpc_url),
            breaker,
            paused: false,
        }
    }

    pub fn slot(&self) -> usize {
        self.clock.slot.try_into().unwrap()
    }

    pub fn epoch_progress(&self) -> u16 {
        let absolute = self.slot() as u64;
        let progress = absolute.rem_euclid(self.epoch_info.slots_in_epoch) as u64;
        ((progress as f64 / self.epoch_info.slots_in_epoch as f64) * 100.0) as u16
    }

    pub fn next(&mut self) {
        self.state.select_next();
        self.scroll_state.next();
        let new_offset = self.state.offset() + ITEM_HEIGHT;
        *self.state.offset_mut() = new_offset;
    }

    pub fn previous(&mut self) {
        self.state.select_previous();
        self.scroll_state.prev();
        let current_offset = self.state.offset();
        let new_offset = if current_offset == 0 {
            0
        } else {
            current_offset - ITEM_HEIGHT
        };
        *self.state.offset_mut() = new_offset;
    }

    // pub fn set_colors(&mut self) {
    //     self.colors = ColorTheme::new(&tailwind::EMERALD)
    // }
}

pub fn start_app(
    simnet_events_rx: Receiver<SimnetEvent>,
    simnet_commands_tx: Sender<SimnetCommand>,
    include_debug_logs: bool,
    deploy_progress_rx: Vec<Receiver<BlockEvent>>,
    remote_rpc_url: &str,
    local_rpc_url: &str,
    breaker: Option<Keypair>,
) -> Result<(), Box<dyn Error>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // create app and run it
    let app = App::new(
        simnet_events_rx,
        simnet_commands_tx,
        include_debug_logs,
        deploy_progress_rx,
        remote_rpc_url,
        local_rpc_url,
        breaker,
    );
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
    let (tx, rx) = unbounded();
    let rpc_api_url = app.local_rpc_url.clone();
    let _ = hiro_system_kit::thread_named("break solana").spawn(move || {
        while let Ok((message, keypair)) = rx.recv() {
            let client =
                RpcClient::new_with_commitment(&rpc_api_url, CommitmentConfig::processed());
            let blockhash = client.get_latest_blockhash().unwrap();
            let transaction = Transaction::new(&[keypair], message, blockhash);
            let _ = client.send_transaction(&transaction).unwrap();
        }
    });

    let mut deployment_completed = false;
    loop {
        let mut selector = Select::new();
        let mut handles = vec![];

        {
            selector.recv(&app.simnet_events_rx);
            if !deployment_completed {
                for rx in app.deploy_progress_rx.iter() {
                    handles.push(selector.recv(rx));
                }
            }

            let oper = selector.try_select();
            if let Ok(oper) = oper {
                match oper.index() {
                    0 => match oper.recv(&app.simnet_events_rx) {
                        Ok(event) => match &event {
                            SimnetEvent::AccountUpdate(dt, _account) => {
                                app.events.push_front((
                                    EventType::Success,
                                    *dt,
                                    event.account_update_msg(),
                                ));
                            }
                            SimnetEvent::PluginLoaded(_) => {
                                app.events.push_front((
                                    EventType::Success,
                                    Local::now(),
                                    event.plugin_loaded_msg(),
                                ));
                            }
                            SimnetEvent::EpochInfoUpdate(epoch_info) => {
                                app.epoch_info = epoch_info.clone();
                                app.events.push_front((
                                    EventType::Success,
                                    Local::now(),
                                    event.epoch_info_update_msg(),
                                ));
                            }
                            SimnetEvent::ClockUpdate(clock) => {
                                app.clock = clock.clone();
                                if app.include_debug_logs {
                                    app.events.push_front((
                                        EventType::Debug,
                                        Local::now(),
                                        event.clock_update_msg(),
                                    ));
                                }
                            }
                            SimnetEvent::ErrorLog(dt, log) => {
                                app.events
                                    .push_front((EventType::Failure, *dt, log.clone()));
                            }
                            SimnetEvent::InfoLog(dt, log) => {
                                app.events.push_front((EventType::Info, *dt, log.clone()));
                            }
                            SimnetEvent::DebugLog(dt, log) => {
                                if app.include_debug_logs {
                                    app.events.push_front((EventType::Debug, *dt, log.clone()));
                                }
                            }
                            SimnetEvent::WarnLog(dt, log) => {
                                app.events
                                    .push_front((EventType::Warning, *dt, log.clone()));
                            }
                            SimnetEvent::TransactionReceived(_dt, _transaction) => {}
                            SimnetEvent::TransactionProcessed(dt, meta, _err) => {
                                if deployment_completed {
                                    for log in meta.logs.iter() {
                                        app.events.push_front((EventType::Debug, *dt, log.clone()));
                                    }
                                }
                                app.successful_transactions += 1;
                            }
                            SimnetEvent::BlockHashExpired => {}
                            SimnetEvent::Aborted(_error) => {
                                break;
                            }
                            SimnetEvent::Ready => {}
                            SimnetEvent::Connected(_) => {}
                            SimnetEvent::Shutdown => {
                                break;
                            }
                            &SimnetEvent::TaggedProfile { .. } => todo!(),
                        },
                        Err(_) => break,
                    },
                    i => match oper.recv(&app.deploy_progress_rx[i - 1]) {
                        Ok(event) => {
                            if let BlockEvent::UpdateProgressBarStatus(update) = event {
                                match update.new_status.status_color {
                                    ProgressBarStatusColor::Yellow => {
                                        app.status_bar_message = Some(format!(
                                            "{}: {}",
                                            update.new_status.status, update.new_status.message
                                        ));
                                    }
                                    ProgressBarStatusColor::Green => {
                                        app.status_bar_message = None;
                                        app.events.push_front((
                                            EventType::Info,
                                            Local::now(),
                                            update.new_status.message,
                                        ));
                                    }
                                    ProgressBarStatusColor::Red => {
                                        app.status_bar_message = None;
                                        app.events.push_front((
                                            EventType::Failure,
                                            Local::now(),
                                            update.new_status.message,
                                        ));
                                    }
                                    ProgressBarStatusColor::Purple => {
                                        app.status_bar_message = None;
                                        app.events.push_front((
                                            EventType::Info,
                                            Local::now(),
                                            update.new_status.message,
                                        ));
                                    }
                                };
                            }
                        }
                        Err(_) => {
                            deployment_completed = true;
                        }
                    },
                }
            }
        }

        if event::poll(Duration::from_millis(5))? {
            if let Event::Key(key_event) = event::read()? {
                if key_event.kind == KeyEventKind::Press {
                    use KeyCode::*;
                    if key_event.modifiers == KeyModifiers::CONTROL && key_event.code == Char('c') {
                        return Ok(());
                    }
                    match key_event.code {
                        Char('q') | Esc => return Ok(()),
                        Down => app.next(),
                        Up => app.previous(),
                        Char('f') | Char('j') => {
                            // Break Solana
                            let sender = app.breaker.as_ref().unwrap();
                            let instruction = system_instruction::transfer(
                                &sender.pubkey(),
                                &Pubkey::new_unique(),
                                100,
                            );
                            let message = Message::new(&[instruction], Some(&sender.pubkey()));
                            let _ = tx.send((message, sender.insecure_clone()));
                        }
                        Char(' ') => {
                            let _ = app
                                .simnet_commands_tx
                                .send(SimnetCommand::UpdateClock(ClockCommand::Toggle));
                            app.paused = !app.paused;
                        }
                        Tab => {
                            let _ = app
                                .simnet_commands_tx
                                .send(SimnetCommand::SlotForward(None));
                        }
                        Char('t') => {
                            let _ = app.simnet_commands_tx.send(
                                SimnetCommand::UpdateBlockProductionMode(
                                    BlockProductionMode::Transaction,
                                ),
                            );
                        }
                        Char('c') => {
                            let _ = app.simnet_commands_tx.send(
                                SimnetCommand::UpdateBlockProductionMode(
                                    BlockProductionMode::Clock,
                                ),
                            );
                        }
                        _ => {}
                    }
                }
            }
        }

        terminal.draw(|f| ui(f, &mut app))?;
    }
    Ok(())
}

fn ui(f: &mut Frame, app: &mut App) {
    let rects = Layout::vertical([
        Constraint::Length(8),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .split(f.area());

    let default_style = Style::new()
        .fg(app.colors.secondary)
        .bg(app.colors.background);
    let chrome = Block::default()
        .style(default_style)
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
        Constraint::Length(56), // Leader Details
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
        .style(default_style)
        .borders(Borders::LEFT)
        .border_style(default_style)
        .border_type(BorderType::Plain);
    f.render_widget(separator, columns[3]);

    render_stats(f, app, columns[3].inner(Margin::new(2, 0)));
}

fn render_stats(f: &mut Frame, app: &mut App, area: Rect) {
    let infos = vec![
        Line::from(vec![
            Span::styled("۬", app.colors.white),
            Span::styled("Surfnet   ", app.colors.gray),
            Span::styled(&app.local_rpc_url, app.colors.white),
        ]),
        Line::from(vec![
            Span::styled("۬", app.colors.white),
            Span::styled("Provider  ", app.colors.gray),
            Span::styled(&app.remote_rpc_url, app.colors.white),
        ]),
        Line::from(vec![Span::styled("۬-", app.colors.gray)]),
        Line::from(vec![
            Span::styled("۬", app.colors.white),
            Span::styled(
                format!("{} ", app.successful_transactions),
                app.colors.accent,
            ),
            Span::styled("transactions processed", app.colors.white),
        ]),
    ];
    let title = Paragraph::new(infos);
    f.render_widget(title.style(app.colors.white), area);
}

fn render_slots(f: &mut Frame, app: &mut App, area: Rect) {
    let line_len = area.width.max(1) as usize;
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

    let (title, color) = if !app.paused {
        let symbol = ["⢎ ", "⠎⠁", "⠊⠑", "⠈⠱", " ⡱", "⢀⡰", "⢄⡠", "⢆⡀"];
        let cursor = symbol[app.slot() % symbol.len()];
        (
            format!("{} Processing incoming transactions", cursor),
            app.colors.accent,
        )
    } else {
        (
            "Transaction processing paused".to_string(),
            app.colors.warning,
        )
    };

    let title = Block::new()
        .padding(Padding::symmetric(4, 4))
        .borders(Borders::NONE)
        .style(Style::new().fg(color))
        .title(Line::from(title));
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
        Constraint::Length(50), // https://txtx.run
    ])
    .split(area);

    let status = match app.status_bar_message {
        Some(ref message) => {
            title_block(message.as_str(), Alignment::Left).style(Style::new().fg(app.colors.gray))
        }
        None => title_block(HELP_TEXT, Alignment::Left).style(Style::new().fg(app.colors.gray)),
    };
    f.render_widget(status, rects[0]);

    let link =
        title_block(SURFPOOL_LINK, Alignment::Right).style(Style::new().fg(app.colors.white));
    f.render_widget(link, rects[1]);
}
