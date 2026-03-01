use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Wrap};
use ratatui::Frame;

use crate::app::{App, Screen, TraverseView};

pub fn render(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_tabs(frame, app, chunks[0]);

    match app.screen {
        Screen::Status => render_status(frame, app, chunks[1]),
        Screen::Traverse => render_traverse(frame, app, chunks[1]),
        Screen::Peers => render_peers(frame, app, chunks[1]),
        Screen::Log => render_log(frame, app, chunks[1]),
        Screen::Info => render_info(frame, app, chunks[1]),
    }

    render_help(frame, app.screen, chunks[2]);
}

fn render_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line> = Screen::ALL
        .iter()
        .enumerate()
        .map(|(i, s)| Line::from(format!(" {} {} ", i + 1, s.label())))
        .collect();

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title(" ZODE CLI "))
        .select(app.screen.index())
        .style(Style::default().fg(Color::Gray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_widget(tabs, area);
}

fn render_help(frame: &mut Frame, screen: Screen, area: Rect) {
    let traverse_hint = match screen {
        Screen::Traverse => " | Enter: drill in | Backspace: back",
        _ => "",
    };
    let text = format!(" q: quit | Tab/1-5: switch screen | ↑↓/jk: scroll{traverse_hint}");
    let help = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, area);
}

fn render_status(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Status ");

    let Some(ref status) = app.status else {
        frame.render_widget(Paragraph::new("Loading...").block(block), area);
        return;
    };

    let mut lines = build_zode_lines(status);
    lines.extend(build_storage_lines(status));
    lines.extend(build_metrics_lines(&status.metrics));
    lines.extend(build_rpc_lines(status));

    app.list_len = 0;
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn build_zode_lines(status: &zode::ZodeStatus) -> Vec<Line<'static>> {
    vec![
        kv_line_owned("ZODE ID:  ", status.zode_id.clone()),
        kv_line_owned("Peers:    ", format!("{}", status.peer_count)),
        Line::from(""),
        kv_line_owned("Programs: ", format!("{}", status.topics.len())),
        Line::from(
            status
                .topics
                .iter()
                .map(|t| format!("  {t}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    ]
}

fn build_storage_lines(status: &zode::ZodeStatus) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        section_header("── Storage ──"),
        kv_line_owned("DB size:  ", format_bytes(status.metrics.db_size_bytes)),
        kv_line_owned(
            "Sectors:  ",
            format!("{}", status.metrics.sectors_stored_total),
        ),
    ]
}

fn build_metrics_lines(m: &zode::MetricsSnapshot) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        section_header("── Metrics ──"),
        kv_line_owned("Stored:     ", format!("{}", m.sectors_stored_total)),
        kv_line_owned(
            "Rejections: ",
            format!(
                "{} (policy: {}, limit: {})",
                m.store_rejections_total, m.policy_rejections, m.limit_rejections,
            ),
        ),
    ]
}

fn build_rpc_lines(status: &zode::ZodeStatus) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(""), section_header("── RPC ──")];

    if status.rpc_enabled {
        let addr = status.rpc_addr.clone().unwrap_or_else(|| "...".into());
        lines.push(kv_line_owned(
            "Status:   ",
            format!("Enabled (listening on {addr})"),
        ));
        let auth = if status.rpc_auth_required {
            "API key required"
        } else {
            "Open"
        };
        lines.push(kv_line_owned("Auth:     ", auth.into()));
        lines.push(kv_line_owned(
            "Requests: ",
            format!("{}", status.metrics.rpc_requests_total),
        ));
    } else {
        lines.push(Line::from(Span::styled(
            "Status:   Disabled",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines
}

fn render_traverse(frame: &mut Frame, app: &mut App, area: Rect) {
    let view = app.traverse.clone();
    match view {
        TraverseView::ProgramList => render_program_list(frame, app, area),
    }
}

fn render_program_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app
        .programs
        .iter()
        .enumerate()
        .map(|(i, pid)| {
            let style = highlight_style(i == app.scroll_offset);
            ListItem::new(Line::from(Span::styled(pid.to_hex(), style)))
        })
        .collect();

    app.list_len = items.len();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Programs ({}) ", items.len()));
    frame.render_widget(List::new(items).block(block), area);
}

fn render_peers(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Connected Zodes ");

    let Some(ref status) = app.status else {
        frame.render_widget(Paragraph::new("Loading...").block(block), area);
        return;
    };

    let mut lines = vec![
        kv_line_owned("Local ZODE: ", status.zode_id.clone()),
        kv_line_owned("Connected:  ", format!("{}", status.peer_count)),
        Line::from(""),
    ];

    if status.connected_peers.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No connected ZODEs.",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (i, peer) in status.connected_peers.iter().enumerate() {
            let style = highlight_style(i == app.scroll_offset);
            lines.push(Line::from(Span::styled(format!("  {peer}"), style)));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "ZODE discovery via GossipSub / bootstrap peers / Kademlia DHT.",
        Style::default().fg(Color::DarkGray),
    )));

    app.list_len = status.connected_peers.len();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_log(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Live Log ({}) ", app.log_entries.len()));

    let visible_height = area.height.saturating_sub(2) as usize;
    let total = app.log_entries.len();
    app.list_len = total;
    let start = total.saturating_sub(visible_height);

    let items: Vec<ListItem> = app
        .log_entries
        .iter()
        .skip(start)
        .map(|entry| {
            ListItem::new(Line::from(Span::styled(
                entry.as_str(),
                log_entry_style(entry),
            )))
        })
        .collect();

    frame.render_widget(List::new(items).block(block), area);
}

fn render_info(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" ZODE Info ");

    let Some(ref status) = app.status else {
        frame.render_widget(Paragraph::new("Loading...").block(block), area);
        return;
    };

    let mut lines = vec![
        kv_line_owned("ZODE ID:        ", status.zode_id.clone()),
        kv_line_owned(
            "DB Size:        ",
            format_bytes(status.metrics.db_size_bytes),
        ),
        kv_line_owned(
            "Sectors Stored: ",
            format!("{}", status.metrics.sectors_stored_total),
        ),
        Line::from(""),
        section_header("── Subscribed Programs ──"),
    ];
    for topic in &status.topics {
        lines.push(Line::from(format!("  {topic}")));
    }
    lines.push(Line::from(""));
    if status.rpc_enabled {
        let addr = status.rpc_addr.clone().unwrap_or_else(|| "...".into());
        let auth = if status.rpc_auth_required {
            "key"
        } else {
            "open"
        };
        lines.push(kv_line_owned(
            "RPC:            ",
            format!("{addr} (auth: {auth})"),
        ));
    } else {
        lines.push(kv_line_owned("RPC:            ", "Disabled".into()));
    }
    lines.push(Line::from(""));
    lines.push(section_header("── Version ──"));
    lines.push(Line::from(format!(
        "  zode-cli v{}",
        env!("CARGO_PKG_VERSION")
    )));

    app.list_len = 0;
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

// ---- shared helpers ----

fn kv_line_owned(label: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(label, Style::default().fg(Color::Yellow)),
        Span::raw(value),
    ])
}

fn section_header(title: &'static str) -> Line<'static> {
    Line::from(Span::styled(title, Style::default().fg(Color::Cyan)))
}

fn highlight_style(selected: bool) -> Style {
    if selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

fn log_entry_style(entry: &str) -> Style {
    use zode::LogLevel;
    match LogLevel::from_log_line(entry) {
        LogLevel::Reject => Style::default().fg(Color::Red),
        LogLevel::Gossip => Style::default().fg(Color::Blue),
        LogLevel::Discovery => Style::default().fg(Color::Blue),
        LogLevel::PeerConnect => Style::default().fg(Color::Green),
        LogLevel::PeerDisconnect => Style::default().fg(Color::Yellow),
        LogLevel::Relay => Style::default().fg(Color::Magenta),
        LogLevel::DialError => Style::default().fg(Color::LightRed),
        LogLevel::Rpc => Style::default().fg(Color::Cyan),
        LogLevel::Shutdown => Style::default().fg(Color::Magenta),
        LogLevel::Normal => Style::default(),
    }
}

use grid_core::format_bytes;
