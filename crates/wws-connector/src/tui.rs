//! Terminal User Interface (TUI) for monitoring the WWS.Connector.
//!
//! Provides a live dashboard showing agent status, network statistics,
//! active tasks, consensus state, and an event log. Built with ratatui
//! and crossterm.
//!
//! Launch with `wws-connector --tui` to run the TUI alongside
//! the connector event loop.

use std::io::{self, Stdout};
use std::sync::Arc;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Table},
    Frame, Terminal,
};
use tokio::sync::RwLock;

use crate::connector::{ConnectorState, ConnectorStatus};

#[derive(Debug, Clone, Default)]
struct FlowSummary {
    injected: usize,
    proposed: usize,
    commits: usize,
    reveals: usize,
    votes: usize,
    selected: usize,
    subtasks: usize,
    assignments: usize,
    results: usize,
    message_events: usize,
    peer_events: usize,
}

/// A log entry for the event log panel.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogEntry {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub category: LogCategory,
    pub message: String,
}

/// Category of a log entry, used for coloring and filtering.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum LogCategory {
    Peer,
    Message,
    Task,
    Vote,
    Epoch,
    Error,
    System,
    Swarm,
}

impl LogCategory {
    /// Short label for display in the event log.
    fn label(&self) -> &'static str {
        match self {
            LogCategory::Peer => "PEER",
            LogCategory::Message => "MSG",
            LogCategory::Task => "TASK",
            LogCategory::Vote => "VOTE",
            LogCategory::Epoch => "EPOCH",
            LogCategory::Error => "ERR",
            LogCategory::System => "SYS",
            LogCategory::Swarm => "SWARM",
        }
    }

    /// Color associated with this log category.
    fn color(&self) -> Color {
        match self {
            LogCategory::Peer => Color::Cyan,
            LogCategory::Message => Color::White,
            LogCategory::Task => Color::Yellow,
            LogCategory::Vote => Color::Green,
            LogCategory::Epoch => Color::Magenta,
            LogCategory::Error => Color::Red,
            LogCategory::System => Color::Blue,
            LogCategory::Swarm => Color::LightCyan,
        }
    }
}

/// Which panel currently has focus for scrolling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusPanel {
    EventLog,
    Tasks,
}

/// The main TUI struct that holds rendering state.
struct SwarmTui {
    /// Shared state from the connector.
    state: Arc<RwLock<ConnectorState>>,
    /// Scroll offset for the event log panel.
    log_scroll: u16,
    /// Scroll offset for the tasks panel.
    task_scroll: u16,
    /// Which panel has focus.
    focus: FocusPanel,
}

impl SwarmTui {
    fn new(state: Arc<RwLock<ConnectorState>>) -> Self {
        Self {
            state,
            log_scroll: 0,
            task_scroll: 0,
            focus: FocusPanel::EventLog,
        }
    }

    /// Take a snapshot of the connector state for rendering.
    /// This minimizes the time we hold the read lock.
    async fn snapshot(&self) -> StateSnapshot {
        let state = self.state.read().await;
        let cascade_status = state.cascade.status();
        let flow_summary = summarize_flow(&state.task_timelines, &state.event_log);
        let (tier1_count, tier2_count, tiern_count, executor_count) = summarize_tiers(&state);

        let current_swarm_id_str = state.current_swarm_id.as_str().to_string();
        let current_swarm_name = state
            .known_swarms
            .get(state.current_swarm_id.as_str())
            .map(|r| r.name.clone())
            .unwrap_or_else(|| current_swarm_id_str.clone());

        let known_swarms: Vec<(String, String, bool, u64, bool)> = state
            .known_swarms
            .values()
            .map(|r| {
                (
                    r.swarm_id.as_str().to_string(),
                    r.name.clone(),
                    r.is_public,
                    r.agent_count,
                    r.joined,
                )
            })
            .collect();

        StateSnapshot {
            agent_id: state.agent_id.to_string(),
            tier: format_tier(&state.my_tier),
            epoch: state.epoch_manager.current_epoch(),
            status: format_status(&state.status),
            status_color: status_color(&state.status),
            parent_id: state.parent_id.as_ref().map(|p| p.to_string()),
            active_tasks: state.task_set.elements(),
            peer_count: state.agent_set.len(),
            swarm_size: state.network_stats.total_agents,
            depth: state.network_stats.hierarchy_depth,
            branching: state.network_stats.branching_factor,
            epoch_duration: state.epoch_manager.epoch_duration_secs(),
            start_time: state.start_time,
            rfp_active: state.rfp_coordinators.len(),
            voting_active: state.voting_engines.len(),
            cascade_completed: cascade_status.completed_subtasks,
            cascade_total: cascade_status.total_subtasks,
            content_items: state.content_store.item_count(),
            event_log: state.event_log.clone(),
            current_swarm_name,
            known_swarms,
            tier1_count,
            tier2_count,
            tiern_count,
            executor_count,
            flow_summary,
        }
    }

    /// Render the full TUI layout into a frame.
    fn render(&self, frame: &mut Frame, snapshot: &StateSnapshot) {
        // Top-level: 4 vertical rows.
        // Row 1: Status + Network (fixed height)
        // Row 2: Swarms + Consensus (fixed height)
        // Row 3: Tasks (fixed height)
        // Row 4: Event Log (flexible, min 6)
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(9),   // Status + Network
                Constraint::Length(10),  // Swarms + Consensus
                Constraint::Length(8),   // Tasks
                Constraint::Min(6),     // Event Log
            ])
            .split(frame.area());

        // Row 1: Status | Network
        let top_row = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(outer[0]);

        self.render_status(frame, top_row[0], snapshot);
        self.render_network(frame, top_row[1], snapshot);

        // Row 2: Swarms | Consensus | Flow
        let mid_row = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(34),
                Constraint::Percentage(33),
                Constraint::Percentage(33),
            ])
            .split(outer[1]);

        self.render_swarms(frame, mid_row[0], snapshot);
        self.render_consensus(frame, mid_row[1], snapshot);
        self.render_flow(frame, mid_row[2], snapshot);

        // Row 3: Tasks (full width)
        self.render_tasks(frame, outer[2], snapshot);

        // Row 4: Event Log (full width)
        self.render_event_log(frame, outer[3], snapshot);
    }

    /// Render the Status panel.
    fn render_status(&self, frame: &mut Frame, area: Rect, snap: &StateSnapshot) {
        let block = Block::default()
            .title(" Status ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White));

        // Truncate agent ID for display.
        let agent_display = if snap.agent_id.len() > 30 {
            format!("{}...", &snap.agent_id[..30])
        } else {
            snap.agent_id.clone()
        };

        let parent_display = snap
            .parent_id
            .as_ref()
            .map(|p| {
                if p.len() > 25 {
                    format!("{}...", &p[..25])
                } else {
                    p.clone()
                }
            })
            .unwrap_or_else(|| "None (root)".to_string());

        let text = vec![
            Line::from(vec![
                Span::styled("  Agent: ", Style::default().fg(Color::Gray)),
                Span::styled(&agent_display, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("  Tier:  ", Style::default().fg(Color::Gray)),
                Span::styled(&snap.tier, Style::default().fg(Color::Cyan)),
            ]),
            Line::from(vec![
                Span::styled("  Epoch: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    snap.epoch.to_string(),
                    Style::default().fg(Color::Magenta),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Status: ", Style::default().fg(Color::Gray)),
                Span::styled(&snap.status, Style::default().fg(snap.status_color)),
            ]),
            Line::from(vec![
                Span::styled("  Parent: ", Style::default().fg(Color::Gray)),
                Span::styled(&parent_display, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("  Swarm: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    &snap.current_swarm_name,
                    Style::default().fg(Color::LightCyan),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Tasks: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("{} active", snap.active_tasks.len()),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
        ];

        let paragraph = Paragraph::new(text).block(block);
        frame.render_widget(paragraph, area);
    }

    /// Render the Network panel.
    fn render_network(&self, frame: &mut Frame, area: Rect, snap: &StateSnapshot) {
        let block = Block::default()
            .title(" Network ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White));

        let uptime = format_uptime(snap.start_time);

        let text = vec![
            Line::from(vec![
                Span::styled("  Peers: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    snap.peer_count.to_string(),
                    Style::default().fg(Color::Green),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Swarm Size: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    snap.swarm_size.to_string(),
                    Style::default().fg(Color::Green),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Depth: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    snap.depth.to_string(),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Branching: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    snap.branching.to_string(),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Epoch Duration: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("{}s", snap.epoch_duration),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Uptime: ", Style::default().fg(Color::Gray)),
                Span::styled(&uptime, Style::default().fg(Color::Cyan)),
            ]),
        ];

        let paragraph = Paragraph::new(text).block(block);
        frame.render_widget(paragraph, area);
    }

    /// Render the Swarms panel showing all known swarms and agent counts.
    fn render_swarms(&self, frame: &mut Frame, area: Rect, snap: &StateSnapshot) {
        let block = Block::default()
            .title(" Swarms ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::LightCyan));

        if snap.known_swarms.is_empty() {
            let text = Paragraph::new(Line::from(vec![Span::styled(
                "  Discovering swarms...",
                Style::default().fg(Color::DarkGray),
            )]))
            .block(block);
            frame.render_widget(text, area);
            return;
        }

        let rows: Vec<Row> = snap
            .known_swarms
            .iter()
            .map(|(_, name, is_public, agent_count, joined)| {
                let type_label = if *is_public { "Public" } else { "Private" };
                let type_color = if *is_public { Color::Green } else { Color::Yellow };
                let status_label = if *joined { "Joined" } else { "Available" };
                let status_color = if *joined { Color::Cyan } else { Color::DarkGray };

                let name_display = if name.len() > 18 {
                    format!("{}...", &name[..18])
                } else {
                    name.clone()
                };

                Row::new(vec![
                    ratatui::widgets::Cell::from(Span::styled(
                        format!("  {}", name_display),
                        Style::default().fg(Color::White),
                    )),
                    ratatui::widgets::Cell::from(Span::styled(
                        type_label,
                        Style::default().fg(type_color),
                    )),
                    ratatui::widgets::Cell::from(Span::styled(
                        agent_count.to_string(),
                        Style::default().fg(Color::White),
                    )),
                    ratatui::widgets::Cell::from(Span::styled(
                        status_label,
                        Style::default().fg(status_color),
                    )),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(40),
                Constraint::Percentage(20),
                Constraint::Percentage(15),
                Constraint::Percentage(25),
            ],
        )
        .block(block)
        .header(
            Row::new(vec!["  Name", "Type", "Agents", "Status"])
                .style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(table, area);
    }

    /// Render the Tasks panel.
    fn render_tasks(&self, frame: &mut Frame, area: Rect, snap: &StateSnapshot) {
        let focus_style = if self.focus == FocusPanel::Tasks {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };

        let block = Block::default()
            .title(" Tasks ")
            .borders(Borders::ALL)
            .border_style(focus_style);

        if snap.active_tasks.is_empty() {
            let text = Paragraph::new(Line::from(vec![Span::styled(
                "  No active tasks",
                Style::default().fg(Color::DarkGray),
            )]))
            .block(block);
            frame.render_widget(text, area);
            return;
        }

        let rows: Vec<Row> = snap
            .active_tasks
            .iter()
            .skip(self.task_scroll as usize)
            .map(|task_id| {
                let short_id = if task_id.len() > 16 {
                    format!("{}...", &task_id[..16])
                } else {
                    task_id.clone()
                };
                Row::new(vec![
                    ratatui::widgets::Cell::from(Span::styled(
                        short_id,
                        Style::default().fg(Color::White),
                    )),
                    ratatui::widgets::Cell::from(Span::styled(
                        "Active",
                        Style::default().fg(Color::Yellow),
                    )),
                    ratatui::widgets::Cell::from(Span::styled(
                        "\u{25b6}",
                        Style::default().fg(Color::Yellow),
                    )),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(55),
                Constraint::Percentage(30),
                Constraint::Percentage(15),
            ],
        )
        .block(block)
        .header(
            Row::new(vec!["  Task ID", "Status", ""])
                .style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(table, area);
    }

    /// Render the Consensus panel.
    fn render_consensus(&self, frame: &mut Frame, area: Rect, snap: &StateSnapshot) {
        let block = Block::default()
            .title(" Consensus ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White));

        let cascade_display = if snap.cascade_total > 0 {
            format!("{}/{} done", snap.cascade_completed, snap.cascade_total)
        } else {
            "idle".to_string()
        };

        let text = vec![
            Line::from(vec![
                Span::styled("  RFP Active: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    snap.rfp_active.to_string(),
                    Style::default().fg(if snap.rfp_active > 0 {
                        Color::Yellow
                    } else {
                        Color::White
                    }),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Voting Active: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    snap.voting_active.to_string(),
                    Style::default().fg(if snap.voting_active > 0 {
                        Color::Green
                    } else {
                        Color::White
                    }),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Cascade: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    &cascade_display,
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Content Items: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    snap.content_items.to_string(),
                    Style::default().fg(Color::White),
                ),
            ]),
        ];

        let paragraph = Paragraph::new(text).block(block);
        frame.render_widget(paragraph, area);
    }

    /// Render the Flow panel for end-to-end process visibility.
    fn render_flow(&self, frame: &mut Frame, area: Rect, snap: &StateSnapshot) {
        let block = Block::default()
            .title(" Flow ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::LightBlue));

        let f = &snap.flow_summary;
        let text = vec![
            Line::from(vec![
                Span::styled("  Tiers: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!(
                        "T1={} T2={} TN={} EX={}",
                        snap.tier1_count, snap.tier2_count, snap.tiern_count, snap.executor_count
                    ),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Tasks: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!(
                        "inj={} prop={} sel={} sub={} res={}",
                        f.injected, f.proposed, f.selected, f.subtasks, f.results
                    ),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Voting: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("commit={} reveal={} votes={}", f.commits, f.reveals, f.votes),
                    Style::default().fg(Color::Green),
                ),
            ]),
            Line::from(vec![
                Span::styled("  P2P: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("msg_events={} peer_events={}", f.message_events, f.peer_events),
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Assignments: ", Style::default().fg(Color::Gray)),
                Span::styled(f.assignments.to_string(), Style::default().fg(Color::White)),
            ]),
        ];

        let paragraph = Paragraph::new(text).block(block);
        frame.render_widget(paragraph, area);
    }

    /// Render the Event Log panel.
    fn render_event_log(&self, frame: &mut Frame, area: Rect, snap: &StateSnapshot) {
        let focus_style = if self.focus == FocusPanel::EventLog {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };

        let block = Block::default()
            .title(" Event Log ")
            .borders(Borders::ALL)
            .border_style(focus_style);

        // Calculate visible area (subtract 2 for borders, 1 for footer).
        let inner_height = area.height.saturating_sub(3) as usize;

        // Build footer line.
        let footer = Line::from(vec![
            Span::styled("  [q] ", Style::default().fg(Color::Yellow)),
            Span::styled("Quit  ", Style::default().fg(Color::Gray)),
            Span::styled("[", Style::default().fg(Color::Yellow)),
            Span::styled("\u{2191}\u{2193}", Style::default().fg(Color::Yellow)),
            Span::styled("] ", Style::default().fg(Color::Yellow)),
            Span::styled("Scroll  ", Style::default().fg(Color::Gray)),
            Span::styled("[Tab] ", Style::default().fg(Color::Yellow)),
            Span::styled("Focus panel", Style::default().fg(Color::Gray)),
        ]);

        if snap.event_log.is_empty() {
            let mut lines = vec![Line::from(Span::styled(
                "  Waiting for events...",
                Style::default().fg(Color::DarkGray),
            ))];
            // Pad to push footer to bottom.
            for _ in 0..inner_height.saturating_sub(1) {
                lines.push(Line::from(""));
            }
            lines.push(footer);

            let paragraph = Paragraph::new(lines).block(block);
            frame.render_widget(paragraph, area);
            return;
        }

        // Auto-scroll: if the user hasn't scrolled up, stay at the bottom.
        let total = snap.event_log.len();
        let max_scroll = total.saturating_sub(inner_height);

        let effective_scroll = if (self.log_scroll as usize) > max_scroll {
            max_scroll
        } else {
            self.log_scroll as usize
        };

        let visible_entries = snap
            .event_log
            .iter()
            .skip(effective_scroll)
            .take(inner_height);

        let mut lines: Vec<Line> = visible_entries
            .map(|entry| {
                let time_str = entry.timestamp.format("%H:%M:%S").to_string();
                Line::from(vec![
                    Span::styled(
                        format!("  [{}] ", time_str),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!("{:<5} ", entry.category.label()),
                        Style::default()
                            .fg(entry.category.color())
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(&entry.message, Style::default().fg(Color::White)),
                ])
            })
            .collect();

        // Pad remaining lines to push footer to bottom.
        let used = lines.len();
        for _ in used..inner_height {
            lines.push(Line::from(""));
        }
        lines.push(footer);

        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(paragraph, area);
    }

    /// Handle keyboard input. Returns `true` if the TUI should exit.
    fn handle_input(&mut self, key: KeyCode, total_log_entries: usize, total_tasks: usize) -> bool {
        match key {
            KeyCode::Char('q') | KeyCode::Char('Q') => return true,
            KeyCode::Tab => {
                self.focus = match self.focus {
                    FocusPanel::EventLog => FocusPanel::Tasks,
                    FocusPanel::Tasks => FocusPanel::EventLog,
                };
            }
            KeyCode::Up => match self.focus {
                FocusPanel::EventLog => {
                    self.log_scroll = self.log_scroll.saturating_sub(1);
                }
                FocusPanel::Tasks => {
                    self.task_scroll = self.task_scroll.saturating_sub(1);
                }
            },
            KeyCode::Down => match self.focus {
                FocusPanel::EventLog => {
                    let max = total_log_entries.saturating_sub(1) as u16;
                    if self.log_scroll < max {
                        self.log_scroll += 1;
                    }
                }
                FocusPanel::Tasks => {
                    let max = total_tasks.saturating_sub(1) as u16;
                    if self.task_scroll < max {
                        self.task_scroll += 1;
                    }
                }
            },
            KeyCode::Home => match self.focus {
                FocusPanel::EventLog => self.log_scroll = 0,
                FocusPanel::Tasks => self.task_scroll = 0,
            },
            KeyCode::End => match self.focus {
                FocusPanel::EventLog => {
                    self.log_scroll = total_log_entries.saturating_sub(1) as u16;
                }
                FocusPanel::Tasks => {
                    self.task_scroll = total_tasks.saturating_sub(1) as u16;
                }
            },
            _ => {}
        }
        false
    }
}

/// Snapshot of connector state used for rendering a single frame.
/// Avoids holding the RwLock during the render phase.
struct StateSnapshot {
    agent_id: String,
    tier: String,
    epoch: u64,
    status: String,
    status_color: Color,
    parent_id: Option<String>,
    active_tasks: Vec<String>,
    peer_count: usize,
    swarm_size: u64,
    depth: u32,
    branching: u32,
    epoch_duration: u64,
    start_time: chrono::DateTime<chrono::Utc>,
    rfp_active: usize,
    voting_active: usize,
    cascade_completed: usize,
    cascade_total: usize,
    content_items: usize,
    event_log: Vec<LogEntry>,
    current_swarm_name: String,
    /// (swarm_id, name, is_public, agent_count, joined)
    known_swarms: Vec<(String, String, bool, u64, bool)>,
    tier1_count: usize,
    tier2_count: usize,
    tiern_count: usize,
    executor_count: usize,
    flow_summary: FlowSummary,
}

fn summarize_tiers(state: &ConnectorState) -> (usize, usize, usize, usize) {
    let mut t1 = 0usize;
    let mut t2 = 0usize;
    let mut tn = 0usize;
    let mut ex = 0usize;

    for tier in state.agent_tiers.values() {
        match tier {
            wws_protocol::Tier::Tier0 | wws_protocol::Tier::Tier1 => t1 += 1,
            wws_protocol::Tier::Tier2 => t2 += 1,
            wws_protocol::Tier::TierN(_) => tn += 1,
            wws_protocol::Tier::Executor => ex += 1,
        }
    }

    if t1 + t2 + tn + ex == 0 {
        match state.my_tier {
            wws_protocol::Tier::Tier0 | wws_protocol::Tier::Tier1 => t1 = 1,
            wws_protocol::Tier::Tier2 => t2 = 1,
            wws_protocol::Tier::TierN(_) => tn = 1,
            wws_protocol::Tier::Executor => ex = 1,
        }
    }

    (t1, t2, tn, ex)
}

fn summarize_flow(
    timelines: &std::collections::HashMap<String, Vec<crate::connector::TaskTimelineEvent>>,
    log: &[LogEntry],
) -> FlowSummary {
    let mut summary = FlowSummary::default();

    for events in timelines.values() {
        for event in events {
            match event.stage.as_str() {
                "injected" => summary.injected += 1,
                "proposed" => summary.proposed += 1,
                "proposal_commit" => summary.commits += 1,
                "proposal_reveal" => summary.reveals += 1,
                "vote_recorded" => summary.votes += 1,
                "plan_selected" => summary.selected += 1,
                "subtask_created" => summary.subtasks += 1,
                "subtask_assigned" | "assigned" => summary.assignments += 1,
                "result_submitted" => summary.results += 1,
                _ => {}
            }
        }
    }

    for entry in log.iter().rev().take(200) {
        match entry.category {
            LogCategory::Message => summary.message_events += 1,
            LogCategory::Peer => summary.peer_events += 1,
            _ => {}
        }
    }

    summary
}

/// Format a Tier enum into a human-readable string.
fn format_tier(tier: &wws_protocol::Tier) -> String {
    match tier {
        wws_protocol::Tier::Tier0 => "Tier0".to_string(),
        wws_protocol::Tier::Tier1 => "Tier1".to_string(),
        wws_protocol::Tier::Tier2 => "Tier2".to_string(),
        wws_protocol::Tier::TierN(n) => format!("Tier{}", n),
        wws_protocol::Tier::Executor => "Executor".to_string(),
    }
}

/// Format a ConnectorStatus enum into a human-readable string.
fn format_status(status: &ConnectorStatus) -> String {
    match status {
        ConnectorStatus::Initializing => "Initializing".to_string(),
        ConnectorStatus::Running => "Running".to_string(),
        ConnectorStatus::InElection => "In Election".to_string(),
        ConnectorStatus::ShuttingDown => "Shutting Down".to_string(),
    }
}

/// Get the color for a ConnectorStatus.
fn status_color(status: &ConnectorStatus) -> Color {
    match status {
        ConnectorStatus::Initializing => Color::Yellow,
        ConnectorStatus::Running => Color::Green,
        ConnectorStatus::InElection => Color::Magenta,
        ConnectorStatus::ShuttingDown => Color::Red,
    }
}

/// Format uptime as a human-readable duration string.
fn format_uptime(start: chrono::DateTime<chrono::Utc>) -> String {
    let elapsed = chrono::Utc::now().signed_duration_since(start);
    let total_secs = elapsed.num_seconds().max(0) as u64;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if hours > 0 {
        format!("{}h {:02}m {:02}s", hours, minutes, seconds)
    } else if minutes > 0 {
        format!("{}m {:02}s", minutes, seconds)
    } else {
        format!("{}s", seconds)
    }
}

/// Set up the terminal for TUI rendering.
fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restore the terminal to its original state.
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Run the TUI event loop.
///
/// This function takes ownership of the shared connector state and renders
/// a live dashboard at approximately 10fps. It handles keyboard input for
/// scrolling, panel focus, and quitting.
///
/// The TUI properly restores terminal state on exit, including on panic.
pub async fn run_tui(state: Arc<RwLock<ConnectorState>>) -> Result<(), anyhow::Error> {
    // Check if we're in a TTY environment before attempting to initialize TUI
    use std::io::IsTerminal;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(anyhow::anyhow!(
            "TUI mode requires a terminal (TTY). Use --no-tui flag when running in background or without a terminal."
        ));
    }

    // Set up a panic hook that restores the terminal.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    let mut terminal = setup_terminal()?;
    let mut tui = SwarmTui::new(state);

    let tick_rate = Duration::from_millis(100); // ~10fps

    loop {
        // Take a snapshot of the state (briefly acquires read lock).
        let snapshot = tui.snapshot().await;

        // Render the frame.
        terminal.draw(|frame| {
            tui.render(frame, &snapshot);
        })?;

        // Poll for keyboard events with the tick rate as timeout.
        if event::poll(tick_rate)? {
            if let Event::Key(key_event) = event::read()? {
                // Only handle key press events (not release or repeat).
                if key_event.kind == KeyEventKind::Press {
                    let should_quit = tui.handle_input(
                        key_event.code,
                        snapshot.event_log.len(),
                        snapshot.active_tasks.len(),
                    );
                    if should_quit {
                        break;
                    }
                }
            }
        }
    }

    // Restore terminal state.
    restore_terminal(&mut terminal)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_summary_counts_task_stages_and_p2p_events() {
        let mut timelines = std::collections::HashMap::new();
        timelines.insert(
            "t1".to_string(),
            vec![
                crate::connector::TaskTimelineEvent {
                    timestamp: chrono::Utc::now(),
                    stage: "injected".to_string(),
                    detail: "".to_string(),
                    actor: None,
                },
                crate::connector::TaskTimelineEvent {
                    timestamp: chrono::Utc::now(),
                    stage: "proposed".to_string(),
                    detail: "".to_string(),
                    actor: None,
                },
                crate::connector::TaskTimelineEvent {
                    timestamp: chrono::Utc::now(),
                    stage: "result_submitted".to_string(),
                    detail: "".to_string(),
                    actor: None,
                },
            ],
        );

        let log = vec![
            LogEntry {
                timestamp: chrono::Utc::now(),
                category: LogCategory::Message,
                message: "m".to_string(),
            },
            LogEntry {
                timestamp: chrono::Utc::now(),
                category: LogCategory::Peer,
                message: "p".to_string(),
            },
        ];

        let summary = summarize_flow(&timelines, &log);
        assert_eq!(summary.injected, 1);
        assert_eq!(summary.proposed, 1);
        assert_eq!(summary.results, 1);
        assert_eq!(summary.message_events, 1);
        assert_eq!(summary.peer_events, 1);
    }
}
