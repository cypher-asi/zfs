use std::collections::VecDeque;

use zfs_core::ProgramId;
use zfs_zode::{LogEvent, MetricsSnapshot, Zode, ZodeStatus};

const MAX_LOG_ENTRIES: usize = 500;

/// Active TUI screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Status,
    Traverse,
    Peers,
    Log,
    Info,
}

impl Screen {
    pub const ALL: [Screen; 5] = [
        Screen::Status,
        Screen::Traverse,
        Screen::Peers,
        Screen::Log,
        Screen::Info,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Screen::Status => "Status",
            Screen::Traverse => "Traverse",
            Screen::Peers => "Peers",
            Screen::Log => "Log",
            Screen::Info => "Info",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&s| s == self).unwrap_or(0)
    }
}

/// Sub-view within the Traverse screen.
#[derive(Debug, Clone)]
pub enum TraverseView {
    ProgramList,
}

/// TUI application state — reads all data from the Zode in-process API.
pub struct App<'z> {
    pub zode: &'z Zode,
    pub screen: Screen,

    // Cached data (refreshed each tick)
    pub status: Option<ZodeStatus>,
    pub metrics: Option<MetricsSnapshot>,

    // Traverse state
    pub traverse: TraverseView,
    pub programs: Vec<ProgramId>,

    // Log buffer
    pub log_entries: VecDeque<String>,
    event_rx: Option<tokio::sync::broadcast::Receiver<LogEvent>>,

    // Scroll position per screen
    pub scroll_offset: usize,
    pub list_len: usize,
}

impl<'z> App<'z> {
    pub fn new(zode: &'z Zode) -> Self {
        let event_rx = Some(zode.subscribe_events());
        Self {
            zode,
            screen: Screen::Status,
            status: None,
            metrics: None,
            traverse: TraverseView::ProgramList,
            programs: Vec::new(),
            log_entries: VecDeque::with_capacity(MAX_LOG_ENTRIES),
            event_rx,
            scroll_offset: 0,
            list_len: 0,
        }
    }

    /// Refresh cached data from the Zode.
    pub async fn refresh(&mut self) {
        let status = self.zode.status();
        self.metrics = Some(status.metrics.clone());

        self.programs = status
            .topics
            .iter()
            .filter_map(|t: &String| {
                let hex = t.strip_prefix("prog/")?;
                ProgramId::from_hex(hex).ok()
            })
            .collect();

        self.status = Some(status);

        self.drain_events();
    }

    fn drain_events(&mut self) {
        if let Some(ref mut rx) = self.event_rx {
            while let Ok(event) = rx.try_recv() {
                let line = event.to_string();
                if self.log_entries.len() >= MAX_LOG_ENTRIES {
                    self.log_entries.pop_front();
                }
                self.log_entries.push_back(line);
            }
        }
    }

    pub fn next_screen(&mut self) {
        let idx = (self.screen.index() + 1) % Screen::ALL.len();
        self.screen = Screen::ALL[idx];
        self.scroll_offset = 0;
    }

    pub fn prev_screen(&mut self) {
        let idx = (self.screen.index() + Screen::ALL.len() - 1) % Screen::ALL.len();
        self.screen = Screen::ALL[idx];
        self.scroll_offset = 0;
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        if self.list_len > 0 && self.scroll_offset < self.list_len.saturating_sub(1) {
            self.scroll_offset += 1;
        }
    }

    /// Handle Enter key — drill into traverse views or no-op on other screens.
    pub fn select(&mut self) {
        // Currently only ProgramList, no deeper drill-down.
    }

    /// Handle Backspace — go back in traverse drill-down.
    pub fn back(&mut self) {
        if self.screen == Screen::Traverse {
            self.traverse = TraverseView::ProgramList;
            self.scroll_offset = 0;
        }
    }
}
