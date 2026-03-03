//! Human-Operator Console for the WWS.Connector.
//!
//! An interactive TUI that allows a human operator (or script piping stdin)
//! to inject tasks into the swarm, view the agent hierarchy tree, monitor
//! active tasks, and observe the event log.
//!
//! Launch with `wws-connector --console`.

use std::io::{self, Stdout};
use std::sync::Arc;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
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
use crate::tui::{LogCategory, LogEntry};
use wws_protocol::{
    ProtocolMethod, SwarmMessage, SwarmTopics, Task, TaskInjectionParams, TaskStatus, Tier,
};

/// A node in the hierarchy tree for display.
#[derive(Debug, Clone)]
pub struct HierarchyNode {
    pub agent_id: String,
    pub display_name: String,
    pub tier: String,
    pub is_self: bool,
    pub children: Vec<HierarchyNode>,
    pub task_count: usize,
    pub last_seen_secs: Option<i64>,
}

/// Snapshot of operator console state for rendering.
#[allow(dead_code)]
struct ConsoleSnapshot {
    agent_id: String,
    tier: String,
    epoch: u64,
    status: String,
    status_color: Color,
    peer_count: usize,
    swarm_size: u64,
    active_tasks: Vec<TaskView>,
    hierarchy: Vec<HierarchyNode>,
    event_log: Vec<LogEntry>,
    current_swarm_name: String,
    flow: FlowSnapshot,
}

#[derive(Debug, Clone, Default)]
struct FlowSnapshot {
    injected: usize,
    proposed: usize,
    commits: usize,
    reveals: usize,
    votes: usize,
    selected: usize,
    subtasks: usize,
    assigned: usize,
    results: usize,
    message_events: usize,
    peer_events: usize,
}

#[derive(Debug, Clone)]
struct TaskView {
    task_id: String,
    status: String,
    description: String,
    assigned_to: String,
    subtask_count: usize,
}

/// The operator console TUI state.
struct OperatorConsole {
    state: Arc<RwLock<ConnectorState>>,
    network_handle: wws_network::SwarmHandle,
    /// Current text in the input field.
    input: String,
    /// Cursor position within the input field.
    cursor_pos: usize,
    /// Command history for up/down arrow navigation.
    history: Vec<String>,
    /// Current position in history (-1 = current input).
    history_pos: Option<usize>,
    /// Scroll offset for the event log panel (reserved for future use).
    #[allow(dead_code)]
    log_scroll: u16,
    /// Scroll offset for the hierarchy panel.
    hierarchy_scroll: u16,
    /// Messages displayed in the console output area.
    console_messages: Vec<(chrono::DateTime<chrono::Utc>, String, Color)>,
}

impl OperatorConsole {
    fn new(
        state: Arc<RwLock<ConnectorState>>,
        network_handle: wws_network::SwarmHandle,
    ) -> Self {
        let mut console_messages = Vec::new();
        console_messages.push((
            chrono::Utc::now(),
            "OpenSwarm Operator Console ready. Type a task description and press Enter to inject it.".to_string(),
            Color::Cyan,
        ));
        console_messages.push((
            chrono::Utc::now(),
            "Commands: /help, /status, /hierarchy, /peers, /quit".to_string(),
            Color::DarkGray,
        ));

        Self {
            state,
            network_handle,
            input: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_pos: None,
            log_scroll: 0,
            hierarchy_scroll: 0,
            console_messages,
        }
    }

    /// Take a snapshot of connector state for rendering.
    async fn snapshot(&self) -> ConsoleSnapshot {
        let state = self.state.read().await;

        let current_swarm_name = state
            .known_swarms
            .get(state.current_swarm_id.as_str())
            .map(|r| r.name.clone())
            .unwrap_or_else(|| state.current_swarm_id.as_str().to_string());

        // Build hierarchy from known agents.
        let hierarchy = build_hierarchy_tree(&state);
        let flow = summarize_flow_snapshot(&state);

        ConsoleSnapshot {
            agent_id: state.agent_id.to_string(),
            tier: format_tier(&state.my_tier),
            epoch: state.epoch_manager.current_epoch(),
            status: format_status(&state.status),
            status_color: status_color(&state.status),
            peer_count: state.member_set.len(),
            swarm_size: state.network_stats.total_agents,
            active_tasks: state
                .task_set
                .elements()
                .iter()
                .map(|task_id| {
                    if let Some(task) = state.task_details.get(task_id) {
                        TaskView {
                            task_id: task.task_id.clone(),
                            status: format_task_status(task.status).to_string(),
                            description: task.description.clone(),
                            assigned_to: task
                                .assigned_to
                                .as_ref()
                                .map(|a| truncate_agent_id(&a.to_string()))
                                .unwrap_or_else(|| "-".to_string()),
                            subtask_count: task.subtasks.len(),
                        }
                    } else {
                        TaskView {
                            task_id: task_id.clone(),
                            status: "Pending".to_string(),
                            description: "(details unavailable)".to_string(),
                            assigned_to: "-".to_string(),
                            subtask_count: 0,
                        }
                    }
                })
                .collect(),
            hierarchy,
            event_log: state.event_log.clone(),
            current_swarm_name,
            flow,
        }
    }

    /// Process a command or task input from the operator.
    async fn process_input(&mut self) {
        let input = self.input.trim().to_string();
        if input.is_empty() {
            return;
        }

        // Save to history.
        self.history.push(input.clone());
        self.history_pos = None;

        if input.starts_with('/') {
            self.process_command(&input).await;
        } else {
            // Treat as a task description to inject.
            self.inject_task(&input).await;
        }

        self.input.clear();
        self.cursor_pos = 0;
    }

    /// Process a slash command.
    async fn process_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
        let command = parts[0];
        let _args = parts.get(1).copied().unwrap_or("");

        match command {
            "/help" => {
                self.add_message(
                    "Available commands:",
                    Color::Cyan,
                );
                self.add_message(
                    "  <text>       - Inject a task with the given description",
                    Color::White,
                );
                self.add_message(
                    "  /status      - Show detailed connector status",
                    Color::White,
                );
                self.add_message(
                    "  /hierarchy   - Show agent hierarchy tree",
                    Color::White,
                );
                self.add_message(
                    "  /agents      - Show known agents and heartbeat age",
                    Color::White,
                );
                self.add_message(
                    "  /peers       - List connected peers",
                    Color::White,
                );
                self.add_message(
                    "  /tasks       - List active tasks",
                    Color::White,
                );
                self.add_message(
                    "  /timeline <task_id> - Show task lifecycle events",
                    Color::White,
                );
                self.add_message(
                    "  /votes [task_id] - Show voting engine status",
                    Color::White,
                );
                self.add_message(
                    "  /flow        - Show flow counters (votes/decompose/results)",
                    Color::White,
                );
                self.add_message(
                    "  /help        - Show this help message",
                    Color::White,
                );
                self.add_message(
                    "  /quit        - Exit the operator console",
                    Color::White,
                );
            }
            "/status" => {
                let (agent_id, tier, epoch, status, members, tasks, content) = {
                    let state = self.state.read().await;
                    (
                        state.agent_id.to_string(),
                        format!("{:?}", state.my_tier),
                        state.epoch_manager.current_epoch(),
                        format!("{:?}", state.status),
                        state.member_set.len(),
                        state.task_set.len(),
                        state.content_store.item_count(),
                    )
                };
                self.add_message(
                    &format!("Agent ID: {}", agent_id),
                    Color::Green,
                );
                self.add_message(
                    &format!("Tier: {} | Epoch: {} | Status: {}", tier, epoch, status),
                    Color::Green,
                );
                self.add_message(
                    &format!("Members: {} | Tasks: {} | Content: {}", members, tasks, content),
                    Color::Green,
                );
            }
            "/peers" => {
                let peers = {
                    let state = self.state.read().await;
                    state.agent_set.elements()
                };
                if peers.is_empty() {
                    self.add_message("No connected peers.", Color::Yellow);
                } else {
                    self.add_message(
                        &format!("Connected peers ({}):", peers.len()),
                        Color::Cyan,
                    );
                    for peer in &peers {
                        self.add_message(&format!("  {}", peer), Color::White);
                    }
                }
            }
            "/tasks" => {
                let tasks = {
                    let state = self.state.read().await;
                    state
                        .task_set
                        .elements()
                        .iter()
                        .map(|task_id| {
                            state
                                .task_details
                                .get(task_id)
                                .cloned()
                                .unwrap_or_else(|| Task {
                                    task_id: task_id.clone(),
                                    parent_task_id: None,
                                    epoch: state.epoch_manager.current_epoch(),
                                    status: TaskStatus::Pending,
                                    description: "(details unavailable)".to_string(),
                                    assigned_to: None,
                                    tier_level: 1,
                                    subtasks: Vec::new(),
                                    created_at: chrono::Utc::now(),
                                    deadline: None,
                                    ..Default::default()
                                })
                        })
                        .collect::<Vec<_>>()
                };
                if tasks.is_empty() {
                    self.add_message("No active tasks.", Color::Yellow);
                } else {
                    self.add_message(
                        &format!("Active tasks ({}):", tasks.len()),
                        Color::Cyan,
                    );
                    for task in &tasks {
                        self.add_message(
                            &format!(
                                "  {} [{}] assigned={} subtasks={} {}",
                                task.task_id,
                                format_task_status(task.status),
                                task
                                    .assigned_to
                                    .as_ref()
                                    .map(|a| truncate_agent_id(&a.to_string()))
                                    .unwrap_or_else(|| "-".to_string()),
                                task.subtasks.len(),
                                task.description
                            ),
                            Color::White,
                        );
                    }
                }
            }
            "/hierarchy" | "/agents" => {
                let hierarchy = {
                    let state = self.state.read().await;
                    build_hierarchy_tree(&state)
                };
                if hierarchy.is_empty() {
                    self.add_message("No hierarchy data available yet.", Color::Yellow);
                } else {
                    self.add_message("Known Agents:", Color::Cyan);
                    for node in &hierarchy {
                        print_hierarchy_node(node, "", true, &mut self.console_messages);
                    }
                }
            }
            "/timeline" => {
                let task_id = parts.get(1).copied().unwrap_or("").trim();
                if task_id.is_empty() {
                    self.add_message("Usage: /timeline <task_id>", Color::Yellow);
                } else {
                    let timeline = {
                        let state = self.state.read().await;
                        state.task_timelines.get(task_id).cloned().unwrap_or_default()
                    };
                    if timeline.is_empty() {
                        self.add_message(&format!("No timeline for task {}", task_id), Color::Yellow);
                    } else {
                        self.add_message(&format!("Timeline for {}:", task_id), Color::Cyan);
                        for event in timeline.iter().rev().take(20).rev() {
                            self.add_message(
                                &format!(
                                    "  {} {} {}",
                                    event.timestamp.format("%H:%M:%S"),
                                    event.stage,
                                    event.detail
                                ),
                                Color::White,
                            );
                        }
                    }
                }
            }
            "/votes" => {
                let selected = parts.get(1).copied().unwrap_or("").trim().to_string();
                let rows = {
                    let state = self.state.read().await;
                    state
                        .voting_engines
                        .iter()
                        .filter(|(task_id, _)| selected.is_empty() || selected == **task_id)
                        .map(|(task_id, voting)| {
                            (
                                task_id.clone(),
                                voting.proposal_count(),
                                voting.ballot_count(),
                            )
                        })
                        .collect::<Vec<_>>()
                };
                if rows.is_empty() {
                    self.add_message("No active voting engines.", Color::Yellow);
                } else {
                    self.add_message("Voting state:", Color::Cyan);
                    for (task, proposals, ballots) in rows {
                        self.add_message(
                            &format!(
                                "  task={} proposals={} ballots={} quorum={}",
                                task,
                                proposals,
                                ballots,
                                ballots >= proposals && ballots > 0
                            ),
                            Color::White,
                        );
                    }
                }
            }
            "/flow" => {
                let flow = {
                    let state = self.state.read().await;
                    summarize_flow_snapshot(&state)
                };
                self.add_message(
                    &format!(
                        "Flow: inj={} prop={} commit={} reveal={} vote={} sel={} sub={} asg={} res={}",
                        flow.injected,
                        flow.proposed,
                        flow.commits,
                        flow.reveals,
                        flow.votes,
                        flow.selected,
                        flow.subtasks,
                        flow.assigned,
                        flow.results
                    ),
                    Color::Cyan,
                );
            }
            "/quit" | "/exit" | "/q" => {
                // Handled in the event loop.
            }
            _ => {
                self.add_message(
                    &format!("Unknown command: {}. Type /help for available commands.", command),
                    Color::Red,
                );
            }
        }
    }

    /// Inject a task into the swarm.
    async fn inject_task(&mut self, description: &str) {
        let mut state = self.state.write().await;
        let epoch = state.epoch_manager.current_epoch();
        let task = Task::new(description.to_string(), 1, epoch);
        let task_id = task.task_id.clone();
        let originator = state.agent_id.clone();
        let swarm_id = state.current_swarm_id.as_str().to_string();

        // Add task to the local task set.
        state.task_set.add(task_id.clone());
        state.task_details.insert(task_id.clone(), task.clone());
        let actor = state.agent_id.to_string();
        state.push_task_timeline_event(
            &task_id,
            "injected",
            format!("Task injected via console: {}", description),
            Some(actor),
        );

        // Log the injection.
        state.push_log(
            LogCategory::Task,
            format!("Operator injected task: {} ({})", task_id, description),
        );

        drop(state);

        // Publish task injection to the swarm network so all peers can process it.
        let inject_params = TaskInjectionParams {
            task: task.clone(),
            originator,
        };

        let msg = SwarmMessage::new(
            ProtocolMethod::TaskInjection.as_str(),
            serde_json::to_value(&inject_params).unwrap_or_default(),
            String::new(),
        );

        if let Ok(data) = serde_json::to_vec(&msg) {
            let topic = SwarmTopics::tasks_for(&swarm_id, 1);
            if let Err(e) = self.network_handle.publish(&topic, data).await {
                tracing::debug!(error = %e, "Failed to publish console task injection");
            }

            let proposals_topic = SwarmTopics::proposals_for(&swarm_id, &task_id);
            let voting_topic = SwarmTopics::voting_for(&swarm_id, &task_id);
            let results_topic = SwarmTopics::results_for(&swarm_id, &task_id);

            let _ = self.network_handle.subscribe(&proposals_topic).await;
            let _ = self.network_handle.subscribe(&voting_topic).await;
            let _ = self.network_handle.subscribe(&results_topic).await;
        }

        self.add_message(
            &format!("Task injected: {}", task_id),
            Color::Green,
        );
        self.add_message(
            &format!("  Description: {}", description),
            Color::White,
        );
    }

    fn add_message(&mut self, msg: &str, color: Color) {
        self.console_messages.push((
            chrono::Utc::now(),
            msg.to_string(),
            color,
        ));
        // Cap at 500 messages.
        if self.console_messages.len() > 500 {
            self.console_messages.remove(0);
        }
    }

    /// Render the full operator console layout.
    fn render(&self, frame: &mut Frame, snapshot: &ConsoleSnapshot) {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // Status bar
                Constraint::Min(8),    // Main area (hierarchy + tasks + log)
                Constraint::Length(5), // Input area
            ])
            .split(frame.area());

        self.render_status_bar(frame, outer[0], snapshot);
        self.render_main_area(frame, outer[1], snapshot);
        self.render_input(frame, outer[2]);
    }

    /// Render the top status bar.
    fn render_status_bar(&self, frame: &mut Frame, area: Rect, snap: &ConsoleSnapshot) {
        let block = Block::default()
            .title(" OpenSwarm Operator Console ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let agent_short = if snap.agent_id.len() > 24 {
            format!("{}...", &snap.agent_id[..24])
        } else {
            snap.agent_id.clone()
        };

        let status_line = Line::from(vec![
            Span::styled("  Agent: ", Style::default().fg(Color::Gray)),
            Span::styled(&agent_short, Style::default().fg(Color::White)),
            Span::styled("  |  Tier: ", Style::default().fg(Color::Gray)),
            Span::styled(&snap.tier, Style::default().fg(Color::Cyan)),
            Span::styled("  |  Epoch: ", Style::default().fg(Color::Gray)),
            Span::styled(snap.epoch.to_string(), Style::default().fg(Color::Magenta)),
            Span::styled("  |  Status: ", Style::default().fg(Color::Gray)),
            Span::styled(&snap.status, Style::default().fg(snap.status_color)),
            Span::styled("  |  Members: ", Style::default().fg(Color::Gray)),
            Span::styled(snap.peer_count.to_string(), Style::default().fg(Color::Green)),
            Span::styled("  |  Swarm: ", Style::default().fg(Color::Gray)),
            Span::styled(&snap.current_swarm_name, Style::default().fg(Color::LightCyan)),
        ]);

        let paragraph = Paragraph::new(status_line).block(block);
        frame.render_widget(paragraph, area);
    }

    /// Render the main area with hierarchy, tasks, and event log.
    fn render_main_area(&self, frame: &mut Frame, area: Rect, snap: &ConsoleSnapshot) {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(40), // Hierarchy
                Constraint::Percentage(60), // Tasks + Log
            ])
            .split(area);

        self.render_hierarchy(frame, columns[0], snap);

        let right_column = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(8),  // Tasks
                Constraint::Length(7),  // Flow
                Constraint::Min(4),     // Console output / Event log
            ])
            .split(columns[1]);

        self.render_tasks(frame, right_column[0], snap);
        self.render_flow(frame, right_column[1], snap);
        self.render_console_output(frame, right_column[2]);
    }

    fn render_flow(&self, frame: &mut Frame, area: Rect, snap: &ConsoleSnapshot) {
        let block = Block::default()
            .title(" Process Flow ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::LightBlue));

        let f = &snap.flow;
        let text = vec![
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
                Span::styled("  Votes: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!("commit={} reveal={} vote={}", f.commits, f.reveals, f.votes),
                    Style::default().fg(Color::Green),
                ),
            ]),
            Line::from(vec![
                Span::styled("  Assign/P2P: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format!(
                        "asg={} msg={} peer={}",
                        f.assigned, f.message_events, f.peer_events
                    ),
                    Style::default().fg(Color::Cyan),
                ),
            ]),
        ];

        frame.render_widget(Paragraph::new(text).block(block), area);
    }

    /// Render the agent hierarchy tree panel.
    fn render_hierarchy(&self, frame: &mut Frame, area: Rect, snap: &ConsoleSnapshot) {
        let block = Block::default()
            .title(" Known Agents ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));

        if snap.hierarchy.is_empty() {
            let text = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  Waiting for agent activity...",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Agents appear when they vote",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(Span::styled(
                    "  or submit task results.",
                    Style::default().fg(Color::DarkGray),
                )),
            ];
            let paragraph = Paragraph::new(text).block(block);
            frame.render_widget(paragraph, area);
            return;
        }

        let mut lines: Vec<Line> = Vec::new();
        for node in &snap.hierarchy {
            render_hierarchy_lines(node, "", true, &mut lines);
        }

        // Apply scroll offset.
        let visible_height = area.height.saturating_sub(2) as usize;
        let scroll = self.hierarchy_scroll as usize;
        let visible: Vec<Line> = lines
            .into_iter()
            .skip(scroll)
            .take(visible_height)
            .collect();

        let paragraph = Paragraph::new(visible).block(block);
        frame.render_widget(paragraph, area);
    }

    /// Render the active tasks panel.
    fn render_tasks(&self, frame: &mut Frame, area: Rect, snap: &ConsoleSnapshot) {
        let block = Block::default()
            .title(format!(" Active Tasks ({}) ", snap.active_tasks.len()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White));

        if snap.active_tasks.is_empty() {
            let text = Paragraph::new(Line::from(Span::styled(
                "  No active tasks. Type a task description below to inject one.",
                Style::default().fg(Color::DarkGray),
            )))
            .block(block);
            frame.render_widget(text, area);
            return;
        }

        let rows: Vec<Row> = snap
            .active_tasks
            .iter()
            .map(|task| {
                let short_id = if task.task_id.len() > 16 {
                    format!("{}...", &task.task_id[..16])
                } else {
                    task.task_id.clone()
                };
                let desc = if task.description.len() > 24 {
                    format!("{}...", &task.description[..24])
                } else {
                    task.description.clone()
                };
                let status_color = match task.status.as_str() {
                    "Pending" => Color::Yellow,
                    "Proposal" => Color::Cyan,
                    "Voting" => Color::Magenta,
                    "In Progress" => Color::Blue,
                    "Completed" => Color::Green,
                    "Failed" | "Rejected" => Color::Red,
                    _ => Color::White,
                };
                Row::new(vec![
                    ratatui::widgets::Cell::from(Span::styled(
                        format!("  {}", short_id),
                        Style::default().fg(Color::White),
                    )),
                    ratatui::widgets::Cell::from(Span::styled(
                        task.status.clone(),
                        Style::default().fg(status_color),
                    )),
                    ratatui::widgets::Cell::from(Span::styled(
                        task.assigned_to.clone(),
                        Style::default().fg(Color::White),
                    )),
                    ratatui::widgets::Cell::from(Span::styled(
                        format!("{} [{} st]", desc, task.subtask_count),
                        Style::default().fg(Color::Gray),
                    )),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(20),
                Constraint::Percentage(16),
                Constraint::Percentage(20),
                Constraint::Percentage(44),
            ],
        )
        .block(block)
        .header(
            Row::new(vec!["  Task ID", "Status", "Assigned", "Description"])
                .style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(table, area);
    }

    /// Render the console output area (mixed console messages + recent events).
    fn render_console_output(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(" Console Output ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White));

        let inner_height = area.height.saturating_sub(2) as usize;

        if self.console_messages.is_empty() {
            let text = Paragraph::new(Line::from(Span::styled(
                "  Waiting for events...",
                Style::default().fg(Color::DarkGray),
            )))
            .block(block);
            frame.render_widget(text, area);
            return;
        }

        // Show the most recent messages that fit.
        let start = self.console_messages.len().saturating_sub(inner_height);
        let visible = &self.console_messages[start..];

        let lines: Vec<Line> = visible
            .iter()
            .map(|(ts, msg, color)| {
                let time_str = ts.format("%H:%M:%S").to_string();
                Line::from(vec![
                    Span::styled(
                        format!("  [{}] ", time_str),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(msg.as_str(), Style::default().fg(*color)),
                ])
            })
            .collect();

        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(paragraph, area);
    }

    /// Render the input area at the bottom.
    fn render_input(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(" Command Input (Enter = inject task, /help = commands, /quit = exit) ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green));

        // Show the input with cursor.
        let input_display = if self.input.is_empty() {
            Line::from(vec![
                Span::styled("  > ", Style::default().fg(Color::Green)),
                Span::styled(
                    "Type a task description or /command...",
                    Style::default().fg(Color::DarkGray),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled("  > ", Style::default().fg(Color::Green)),
                Span::styled(&self.input, Style::default().fg(Color::White)),
            ])
        };

        let hint_line = Line::from(vec![
            Span::styled(
                "  Ctrl+C or /quit to exit  |  Up/Down for history  |  Enter to submit",
                Style::default().fg(Color::DarkGray),
            ),
        ]);

        let paragraph = Paragraph::new(vec![
            Line::from(""),
            input_display,
            hint_line,
        ])
        .block(block);
        frame.render_widget(paragraph, area);

        // Position cursor.
        let cursor_x = area.x + 4 + self.cursor_pos as u16;
        let cursor_y = area.y + 2;
        frame.set_cursor_position((cursor_x, cursor_y));
    }

    /// Handle keyboard input. Returns `true` if the console should exit.
    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        match (code, modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => return true,
            (KeyCode::Char(c), _) => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
            }
            (KeyCode::Backspace, _) => {
                if self.cursor_pos > 0 {
                    self.input.remove(self.cursor_pos - 1);
                    self.cursor_pos -= 1;
                }
            }
            (KeyCode::Delete, _) => {
                if self.cursor_pos < self.input.len() {
                    self.input.remove(self.cursor_pos);
                }
            }
            (KeyCode::Left, _) => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                }
            }
            (KeyCode::Right, _) => {
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos += 1;
                }
            }
            (KeyCode::Home, _) => {
                self.cursor_pos = 0;
            }
            (KeyCode::End, _) => {
                self.cursor_pos = self.input.len();
            }
            (KeyCode::Up, _) => {
                if !self.history.is_empty() {
                    let pos = match self.history_pos {
                        Some(p) if p > 0 => p - 1,
                        Some(p) => p,
                        None => self.history.len() - 1,
                    };
                    self.history_pos = Some(pos);
                    self.input = self.history[pos].clone();
                    self.cursor_pos = self.input.len();
                }
            }
            (KeyCode::Down, _) => {
                if let Some(pos) = self.history_pos {
                    if pos + 1 < self.history.len() {
                        let new_pos = pos + 1;
                        self.history_pos = Some(new_pos);
                        self.input = self.history[new_pos].clone();
                        self.cursor_pos = self.input.len();
                    } else {
                        self.history_pos = None;
                        self.input.clear();
                        self.cursor_pos = 0;
                    }
                }
            }
            (KeyCode::Enter, _) => {
                // Handled by caller (needs async).
            }
            (KeyCode::PageUp, _) => {
                self.hierarchy_scroll = self.hierarchy_scroll.saturating_sub(5);
            }
            (KeyCode::PageDown, _) => {
                self.hierarchy_scroll += 5;
            }
            _ => {}
        }
        false
    }
}

/// Build a hierarchy tree from the connector state.
fn build_hierarchy_tree(state: &ConnectorState) -> Vec<HierarchyNode> {
    let mut nodes: std::collections::HashMap<String, HierarchyNode> = std::collections::HashMap::new();
    for agent_id in state.active_member_ids(Duration::from_secs(180)) {
        let last_seen_secs = state.member_last_seen.get(&agent_id).map(|ts| {
            chrono::Utc::now()
                .signed_duration_since(*ts)
                .num_seconds()
                .max(0)
        });
        let tier = state
            .agent_tiers
            .get(&agent_id)
            .map(format_tier)
            .unwrap_or_else(|| {
                if agent_id == state.agent_id.to_string() {
                    format_tier(&state.my_tier)
                } else {
                    "Peer".to_string()
                }
            });
        let task_count = state
            .task_details
            .values()
            .filter(|task| task.assigned_to.as_ref().map(|a| a.to_string()) == Some(agent_id.clone()))
            .count();

        nodes.insert(
            agent_id.clone(),
            HierarchyNode {
                display_name: truncate_agent_id(&agent_id),
                agent_id: agent_id.clone(),
                tier,
                is_self: agent_id == state.agent_id.to_string(),
                children: Vec::new(),
                task_count,
                last_seen_secs,
            },
        );
    }

    let mut roots: Vec<String> = Vec::new();
    let keys: Vec<String> = nodes.keys().cloned().collect();
    for agent_id in keys {
        if let Some(parent) = state.agent_parents.get(&agent_id) {
            if nodes.contains_key(parent) {
                let child = nodes.get(&agent_id).cloned();
                if let (Some(parent_node), Some(child_node)) = (nodes.get_mut(parent), child) {
                    parent_node.children.push(child_node);
                }
                continue;
            }
        }
        roots.push(agent_id);
    }

    roots.sort();
    roots
        .into_iter()
        .filter_map(|id| nodes.get(&id).cloned())
        .collect()
}

fn summarize_flow_snapshot(state: &ConnectorState) -> FlowSnapshot {
    let mut flow = FlowSnapshot::default();

    for events in state.task_timelines.values() {
        for event in events {
            match event.stage.as_str() {
                "injected" => flow.injected += 1,
                "proposed" => flow.proposed += 1,
                "proposal_commit" => flow.commits += 1,
                "proposal_reveal" => flow.reveals += 1,
                "vote_recorded" => flow.votes += 1,
                "plan_selected" => flow.selected += 1,
                "subtask_created" => flow.subtasks += 1,
                "subtask_assigned" | "assigned" => flow.assigned += 1,
                "result_submitted" => flow.results += 1,
                _ => {}
            }
        }
    }

    for entry in state.event_log.iter().rev().take(200) {
        match entry.category {
            LogCategory::Message => flow.message_events += 1,
            LogCategory::Peer => flow.peer_events += 1,
            _ => {}
        }
    }

    flow
}

/// Render hierarchy tree into display lines.
fn render_hierarchy_lines(
    node: &HierarchyNode,
    prefix: &str,
    is_last: bool,
    lines: &mut Vec<Line<'static>>,
) {
    let branch = if prefix.is_empty() {
        "  ".to_string()
    } else if is_last {
        "  └── ".to_string()
    } else {
        "  ├── ".to_string()
    };

    let tier_color = match node.tier.as_str() {
        "Tier1" => Color::Red,
        "Tier2" => Color::Yellow,
        t if t.starts_with("Tier") => Color::LightYellow,
        "Executor" => Color::Green,
        "Agent" => Color::Cyan,
        "Peer" => Color::Cyan,
        _ => Color::White,
    };

    let self_marker = if node.is_self { " (you)" } else { "" };
    let last_seen = node
        .last_seen_secs
        .map(|s| format!(" [{}s ago]", s))
        .unwrap_or_else(|| " [never]".to_string());
    let task_info = if node.task_count > 0 {
        format!(" [{}t]", node.task_count)
    } else {
        String::new()
    };

    lines.push(Line::from(vec![
        Span::styled(branch, Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("[{}]", node.tier),
            Style::default().fg(tier_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {}", node.display_name),
            Style::default().fg(if node.is_self {
                Color::White
            } else {
                Color::Gray
            }),
        ),
        Span::styled(
            self_marker.to_string(),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        Span::styled(last_seen, Style::default().fg(Color::DarkGray)),
        Span::styled(task_info, Style::default().fg(Color::Yellow)),
    ]));

    let child_prefix = if prefix.is_empty() {
        "  ".to_string()
    } else if is_last {
        format!("{}     ", prefix)
    } else {
        format!("{}  │  ", prefix)
    };

    for (i, child) in node.children.iter().enumerate() {
        let is_child_last = i == node.children.len() - 1;
        render_hierarchy_lines(child, &child_prefix, is_child_last, lines);
    }
}

/// Print hierarchy node to console messages (for /hierarchy command).
fn print_hierarchy_node(
    node: &HierarchyNode,
    prefix: &str,
    is_last: bool,
    messages: &mut Vec<(chrono::DateTime<chrono::Utc>, String, Color)>,
) {
    let branch = if prefix.is_empty() {
        "".to_string()
    } else if is_last {
        format!("{}└── ", prefix)
    } else {
        format!("{}├── ", prefix)
    };

    let self_marker = if node.is_self { " (you)" } else { "" };
    let last_seen = node
        .last_seen_secs
        .map(|s| format!(" [{}s ago]", s))
        .unwrap_or_else(|| " [never]".to_string());
    messages.push((
        chrono::Utc::now(),
        format!(
            "  {}[{}] {}{}{}",
            branch, node.tier, node.display_name, self_marker, last_seen
        ),
        if node.is_self { Color::Green } else { Color::White },
    ));

    let child_prefix = if prefix.is_empty() {
        "".to_string()
    } else if is_last {
        format!("{}    ", prefix)
    } else {
        format!("{}│   ", prefix)
    };

    for (i, child) in node.children.iter().enumerate() {
        let is_child_last = i == node.children.len() - 1;
        print_hierarchy_node(child, &child_prefix, is_child_last, messages);
    }
}

fn truncate_agent_id(id: &str) -> String {
    if id.len() > 28 {
        format!("{}...{}", &id[..16], &id[id.len() - 8..])
    } else {
        id.to_string()
    }
}

fn format_tier(tier: &Tier) -> String {
    match tier {
        Tier::Tier1 => "Tier1".to_string(),
        Tier::Tier2 => "Tier2".to_string(),
        Tier::TierN(n) => format!("Tier{}", n),
        Tier::Executor => "Executor".to_string(),
    }
}

fn format_task_status(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "Pending",
        TaskStatus::ProposalPhase => "Proposal",
        TaskStatus::VotingPhase => "Voting",
        TaskStatus::InProgress => "In Progress",
        TaskStatus::Completed => "Completed",
        TaskStatus::Failed => "Failed",
        TaskStatus::Rejected => "Rejected",
        TaskStatus::PendingReview => "Pending Review",
    }
}

fn format_status(status: &ConnectorStatus) -> String {
    match status {
        ConnectorStatus::Initializing => "Initializing".to_string(),
        ConnectorStatus::Running => "Running".to_string(),
        ConnectorStatus::InElection => "In Election".to_string(),
        ConnectorStatus::ShuttingDown => "Shutting Down".to_string(),
    }
}

fn status_color(status: &ConnectorStatus) -> Color {
    match status {
        ConnectorStatus::Initializing => Color::Yellow,
        ConnectorStatus::Running => Color::Green,
        ConnectorStatus::InElection => Color::Magenta,
        ConnectorStatus::ShuttingDown => Color::Red,
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

/// Run the operator console event loop.
///
/// This is the main entry point for the operator console. It provides an
/// interactive TUI where a human operator can:
/// - Type task descriptions and press Enter to inject them into the swarm
/// - View the agent hierarchy tree
/// - Monitor active tasks and events
/// - Use slash commands for additional operations
pub async fn run_operator_console(
    state: Arc<RwLock<ConnectorState>>,
    network_handle: wws_network::SwarmHandle,
) -> Result<(), anyhow::Error> {
    use std::io::IsTerminal;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(anyhow::anyhow!(
            "Operator console requires a terminal (TTY)."
        ));
    }

    // Set up panic hook to restore terminal.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    let mut terminal = setup_terminal()?;
    let mut console = OperatorConsole::new(state, network_handle);

    let tick_rate = Duration::from_millis(100); // ~10fps

    loop {
        // Take a snapshot.
        let snapshot = console.snapshot().await;

        // Render.
        terminal.draw(|frame| {
            console.render(frame, &snapshot);
        })?;

        // Poll for keyboard events.
        if event::poll(tick_rate)? {
            if let Event::Key(key_event) = event::read()? {
                if key_event.kind == KeyEventKind::Press {
                    // Check for quit commands first.
                    if key_event.code == KeyCode::Enter {
                        // Check if input is a quit command.
                        let trimmed = console.input.trim().to_string();
                        if trimmed == "/quit" || trimmed == "/exit" || trimmed == "/q" {
                            break;
                        }
                        console.process_input().await;
                    } else if console.handle_key(key_event.code, key_event.modifiers) {
                        break; // Ctrl+C
                    }
                }
            }
        }
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wws_network::{
        NetworkEvent, SwarmHost, SwarmHostConfig, discovery::DiscoveryConfig,
        transport::TransportConfig,
    };
    use wws_protocol::SwarmTopics;
    use std::time::Duration;
    use tokio::time::timeout;

    #[test]
    fn hierarchy_building_shows_only_known_agents() {
        use wws_hierarchy::{EpochManager, GeoCluster, PyramidAllocator, SuccessionManager};
        use wws_protocol::{AgentId, SwarmId, Tier};
        use wws_state::{ContentStore, GranularityAlgorithm, MerkleDag, OrSet};

        let mut state = ConnectorState {
            agent_id: AgentId::new("did:swarm:connector-self".to_string()),
            status: ConnectorStatus::Running,
            epoch_manager: EpochManager::default(),
            pyramid: PyramidAllocator::default(),
            election: None,
            geo_cluster: GeoCluster::default(),
            succession: SuccessionManager::new(),
            rfp_coordinators: std::collections::HashMap::new(),
            voting_engines: std::collections::HashMap::new(),
            cascade: wws_consensus::CascadeEngine::new(),
            task_set: OrSet::new("seed".to_string()),
            task_details: std::collections::HashMap::new(),
            task_timelines: std::collections::HashMap::new(),
            agent_set: OrSet::new("seed".to_string()),
            member_set: OrSet::new("seed".to_string()),
            member_last_seen: std::collections::HashMap::new(),
            agent_names: std::collections::HashMap::new(),
            agent_activity: std::collections::HashMap::new(),
            task_vote_requirements: std::collections::HashMap::new(),
            member_last_task_poll: std::collections::HashMap::new(),
            member_last_result: std::collections::HashMap::new(),
            task_result_text: std::collections::HashMap::new(),
            pending_plan_reveals: std::collections::HashMap::new(),
            merkle_dag: MerkleDag::new(),
            content_store: ContentStore::new(),
            granularity: GranularityAlgorithm::default(),
            my_tier: Tier::Executor,
            parent_id: None,
            agent_tiers: std::collections::HashMap::new(),
            agent_parents: std::collections::HashMap::new(),
            current_layout: None,
            subordinates: std::collections::HashMap::new(),
            task_results: std::collections::HashMap::new(),
            network_stats: wws_protocol::NetworkStats {
                total_agents: 0,
                hierarchy_depth: 1,
                branching_factor: 10,
                current_epoch: 1,
                my_tier: Tier::Executor,
                subordinate_count: 0,
                parent_id: None,
            },
            event_log: Vec::new(),
            message_trace: Vec::new(),
            start_time: chrono::Utc::now(),
            current_swarm_id: SwarmId::new("public".to_string()),
            known_swarms: std::collections::HashMap::new(),
            swarm_token: None,
            active_holons: std::collections::HashMap::new(),
            deliberation_messages: std::collections::HashMap::new(),
            ballot_records: std::collections::HashMap::new(),
            irv_rounds: std::collections::HashMap::new(),
            board_acceptances: std::collections::HashMap::new(),
            name_registry: std::collections::HashMap::new(),
            inbox: Vec::new(),
            inject_rate_limiter: std::collections::HashMap::new(),
            reputation_ledgers: std::collections::HashMap::new(),
            rep_event_rate_limiter: std::collections::HashMap::new(),
            pending_key_rotations: std::collections::HashMap::new(),
            pending_revocations: std::collections::HashMap::new(),
            guardian_designations: std::collections::HashMap::new(),
            guardian_votes: std::collections::HashMap::new(),
            receipts: std::collections::HashMap::new(),
            clarifications: std::collections::HashMap::new(),
        };

        state.mark_member_seen("did:swarm:agent-1");
        state.mark_member_seen("did:swarm:agent-2");

        let tree = build_hierarchy_tree(&state);
        assert_eq!(tree.len(), 3);
        assert!(tree.iter().any(|n| n.is_self));
        assert!(tree
            .iter()
            .filter(|n| !n.is_self)
            .all(|n| n.last_seen_secs.is_some()));
    }

    #[test]
    fn flow_summary_counts_votes_decomposition_results() {
        use wws_hierarchy::{EpochManager, GeoCluster, PyramidAllocator, SuccessionManager};
        use wws_protocol::{AgentId, SwarmId, Tier};
        use wws_state::{ContentStore, GranularityAlgorithm, MerkleDag, OrSet};

        let mut state = ConnectorState {
            agent_id: AgentId::new("did:swarm:flow-test".to_string()),
            status: ConnectorStatus::Running,
            epoch_manager: EpochManager::default(),
            pyramid: PyramidAllocator::default(),
            election: None,
            geo_cluster: GeoCluster::default(),
            succession: SuccessionManager::new(),
            rfp_coordinators: std::collections::HashMap::new(),
            voting_engines: std::collections::HashMap::new(),
            cascade: wws_consensus::CascadeEngine::new(),
            task_set: OrSet::new("seed".to_string()),
            task_details: std::collections::HashMap::new(),
            task_timelines: std::collections::HashMap::new(),
            agent_set: OrSet::new("seed".to_string()),
            member_set: OrSet::new("seed".to_string()),
            member_last_seen: std::collections::HashMap::new(),
            agent_names: std::collections::HashMap::new(),
            agent_activity: std::collections::HashMap::new(),
            task_vote_requirements: std::collections::HashMap::new(),
            member_last_task_poll: std::collections::HashMap::new(),
            member_last_result: std::collections::HashMap::new(),
            task_result_text: std::collections::HashMap::new(),
            pending_plan_reveals: std::collections::HashMap::new(),
            merkle_dag: MerkleDag::new(),
            content_store: ContentStore::new(),
            granularity: GranularityAlgorithm::default(),
            my_tier: Tier::Tier1,
            parent_id: None,
            agent_tiers: std::collections::HashMap::new(),
            agent_parents: std::collections::HashMap::new(),
            current_layout: None,
            subordinates: std::collections::HashMap::new(),
            task_results: std::collections::HashMap::new(),
            network_stats: wws_protocol::NetworkStats {
                total_agents: 1,
                hierarchy_depth: 1,
                branching_factor: 10,
                current_epoch: 1,
                my_tier: Tier::Tier1,
                subordinate_count: 0,
                parent_id: None,
            },
            event_log: Vec::new(),
            message_trace: Vec::new(),
            start_time: chrono::Utc::now(),
            current_swarm_id: SwarmId::new("public".to_string()),
            known_swarms: std::collections::HashMap::new(),
            swarm_token: None,
            active_holons: std::collections::HashMap::new(),
            deliberation_messages: std::collections::HashMap::new(),
            ballot_records: std::collections::HashMap::new(),
            irv_rounds: std::collections::HashMap::new(),
            board_acceptances: std::collections::HashMap::new(),
            name_registry: std::collections::HashMap::new(),
            inbox: Vec::new(),
            inject_rate_limiter: std::collections::HashMap::new(),
            reputation_ledgers: std::collections::HashMap::new(),
            rep_event_rate_limiter: std::collections::HashMap::new(),
            pending_key_rotations: std::collections::HashMap::new(),
            pending_revocations: std::collections::HashMap::new(),
            guardian_designations: std::collections::HashMap::new(),
            guardian_votes: std::collections::HashMap::new(),
            receipts: std::collections::HashMap::new(),
            clarifications: std::collections::HashMap::new(),
        };

        state.push_task_timeline_event("t1", "injected", "", None);
        state.push_task_timeline_event("t1", "proposed", "", None);
        state.push_task_timeline_event("t1", "proposal_commit", "", None);
        state.push_task_timeline_event("t1", "proposal_reveal", "", None);
        state.push_task_timeline_event("t1", "vote_recorded", "", None);
        state.push_task_timeline_event("t1", "plan_selected", "", None);
        state.push_task_timeline_event("t1", "subtask_created", "", None);
        state.push_task_timeline_event("t1", "subtask_assigned", "", None);
        state.push_task_timeline_event("t1", "result_submitted", "", None);
        state.push_log(LogCategory::Message, "msg".to_string());
        state.push_log(LogCategory::Peer, "peer".to_string());

        let flow = summarize_flow_snapshot(&state);
        assert_eq!(flow.injected, 1);
        assert_eq!(flow.proposed, 1);
        assert_eq!(flow.commits, 1);
        assert_eq!(flow.reveals, 1);
        assert_eq!(flow.votes, 1);
        assert_eq!(flow.selected, 1);
        assert_eq!(flow.subtasks, 1);
        assert_eq!(flow.assigned, 1);
        assert_eq!(flow.results, 1);
        assert_eq!(flow.message_events, 1);
        assert_eq!(flow.peer_events, 1);
    }

    #[tokio::test]
    async fn console_inject_task_publishes_to_swarm() {
        let cfg = SwarmHostConfig {
            listen_addr: "/ip4/127.0.0.1/tcp/0".parse().expect("valid listen addr"),
            transport: TransportConfig::default(),
            discovery: DiscoveryConfig {
                mdns_enabled: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let (host_a, handle_a, mut events_a) = SwarmHost::new(cfg.clone()).expect("host A");
        let (host_b, handle_b, mut events_b) = SwarmHost::new(cfg).expect("host B");

        let task_a = tokio::spawn(async move { host_a.run().await });
        let task_b = tokio::spawn(async move { host_b.run().await });

        let mut addr_b = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            match timeout(Duration::from_millis(250), events_b.recv()).await {
                Ok(Some(NetworkEvent::Listening(addr))) => {
                    addr_b = Some(addr);
                    break;
                }
                Ok(Some(_)) | Ok(None) | Err(_) => {}
            }
        }
        let addr_b = addr_b.expect("node B listening address");

        handle_a.dial(addr_b).await.expect("dial B");

        let connected = timeout(Duration::from_secs(10), async {
            loop {
                if let Some(NetworkEvent::PeerConnected(_)) = events_a.recv().await {
                    return true;
                }
            }
        })
        .await
        .expect("connection event timeout");
        assert!(connected);

        let task_topic = SwarmTopics::tasks_for("public", 1);
        handle_b
            .subscribe(&task_topic)
            .await
            .expect("subscribe B task topic");
        tokio::time::sleep(Duration::from_secs(1)).await;

        use wws_hierarchy::{EpochManager, GeoCluster, PyramidAllocator, SuccessionManager};
        use wws_protocol::{AgentId, SwarmId, Tier};
        use wws_state::{ContentStore, GranularityAlgorithm, MerkleDag, OrSet};

        let state = ConnectorState {
            agent_id: AgentId::new("did:swarm:console-test".to_string()),
            status: ConnectorStatus::Running,
            epoch_manager: EpochManager::default(),
            pyramid: PyramidAllocator::default(),
            election: None,
            geo_cluster: GeoCluster::default(),
            succession: SuccessionManager::new(),
            rfp_coordinators: std::collections::HashMap::new(),
            voting_engines: std::collections::HashMap::new(),
            cascade: wws_consensus::CascadeEngine::new(),
            task_set: OrSet::new("seed".to_string()),
            task_details: std::collections::HashMap::new(),
            task_timelines: std::collections::HashMap::new(),
            agent_set: OrSet::new("seed".to_string()),
            member_set: OrSet::new("seed".to_string()),
            member_last_seen: std::collections::HashMap::new(),
            agent_names: std::collections::HashMap::new(),
            agent_activity: std::collections::HashMap::new(),
            task_vote_requirements: std::collections::HashMap::new(),
            member_last_task_poll: std::collections::HashMap::new(),
            member_last_result: std::collections::HashMap::new(),
            task_result_text: std::collections::HashMap::new(),
            pending_plan_reveals: std::collections::HashMap::new(),
            merkle_dag: MerkleDag::new(),
            content_store: ContentStore::new(),
            granularity: GranularityAlgorithm::default(),
            my_tier: Tier::Tier1,
            parent_id: None,
            agent_tiers: std::collections::HashMap::new(),
            agent_parents: std::collections::HashMap::new(),
            current_layout: None,
            subordinates: std::collections::HashMap::new(),
            task_results: std::collections::HashMap::new(),
            network_stats: wws_protocol::NetworkStats {
                total_agents: 1,
                hierarchy_depth: 1,
                branching_factor: 10,
                current_epoch: 1,
                my_tier: Tier::Tier1,
                subordinate_count: 0,
                parent_id: None,
            },
            event_log: Vec::new(),
            message_trace: Vec::new(),
            start_time: chrono::Utc::now(),
            current_swarm_id: SwarmId::new("public".to_string()),
            known_swarms: std::collections::HashMap::new(),
            swarm_token: None,
            active_holons: std::collections::HashMap::new(),
            deliberation_messages: std::collections::HashMap::new(),
            ballot_records: std::collections::HashMap::new(),
            irv_rounds: std::collections::HashMap::new(),
            board_acceptances: std::collections::HashMap::new(),
            name_registry: std::collections::HashMap::new(),
            inbox: Vec::new(),
            inject_rate_limiter: std::collections::HashMap::new(),
            reputation_ledgers: std::collections::HashMap::new(),
            rep_event_rate_limiter: std::collections::HashMap::new(),
            pending_key_rotations: std::collections::HashMap::new(),
            pending_revocations: std::collections::HashMap::new(),
            guardian_designations: std::collections::HashMap::new(),
            guardian_votes: std::collections::HashMap::new(),
            receipts: std::collections::HashMap::new(),
            clarifications: std::collections::HashMap::new(),
        };

        let mut console = OperatorConsole::new(Arc::new(RwLock::new(state)), handle_a.clone());
        console.inject_task("console injected task").await;

        let received = timeout(Duration::from_secs(10), async {
            loop {
                match events_b.recv().await {
                    Some(NetworkEvent::MessageReceived { topic, data, .. }) if topic == task_topic => {
                        return Some(data);
                    }
                    Some(_) => continue,
                    None => return None,
                }
            }
        })
        .await
        .expect("message receive timeout")
        .expect("task injection message");

        let msg: wws_protocol::SwarmMessage =
            serde_json::from_slice(&received).expect("valid swarm message");
        assert_eq!(msg.method, wws_protocol::ProtocolMethod::TaskInjection.as_str());

        task_a.abort();
        task_b.abort();
    }
}
