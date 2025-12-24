use crate::stats::ProxyStats;
use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame, Terminal,
};
use std::io;
use std::time::{Duration, SystemTime};

/// Statistics dashboard state
pub struct Dashboard {
    url: String,
    interval: Duration,
    stats: Vec<ProxyStats>,
    last_update: Option<SystemTime>,
    error_message: Option<String>,
}

impl Dashboard {
    pub fn new(url: String, interval: u64) -> Self {
        Self {
            url,
            interval: Duration::from_secs(interval),
            stats: Vec::new(),
            last_update: None,
            error_message: None,
        }
    }

    /// Fetch statistics from server
    async fn fetch_stats(&mut self) -> Result<()> {
        let url = format!("{}/api/stats", self.url);
        let response = reqwest::get(&url)
            .await
            .context("Failed to fetch statistics")?;

        if !response.status().is_success() {
            anyhow::bail!("Server returned error: {}", response.status());
        }

        self.stats = response
            .json::<Vec<ProxyStats>>()
            .await
            .context("Failed to parse statistics")?;
        self.last_update = Some(SystemTime::now());
        self.error_message = None;

        Ok(())
    }

    /// Render the dashboard UI
    fn render(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Stats table
                Constraint::Length(3), // Footer
            ])
            .split(f.area());

        // Render header
        self.render_header(f, chunks[0]);

        // Render stats table
        self.render_stats_table(f, chunks[1]);

        // Render footer
        self.render_footer(f, chunks[2]);
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let title = if let Some(last_update) = self.last_update {
            let elapsed = SystemTime::now()
                .duration_since(last_update)
                .unwrap_or_default();
            format!(
                "TLS Tunnel Statistics - {} - Last update: {}s ago",
                self.url,
                elapsed.as_secs()
            )
        } else {
            format!("TLS Tunnel Statistics - {} - Waiting for data...", self.url)
        };

        let header = Paragraph::new(title)
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .block(Block::default().borders(Borders::ALL));

        f.render_widget(header, area);
    }

    fn render_stats_table(&self, f: &mut Frame, area: Rect) {
        if let Some(error) = &self.error_message {
            let error_text = Paragraph::new(format!("Error: {}", error))
                .style(Style::default().fg(Color::Red))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Connection Error"),
                );
            f.render_widget(error_text, area);
            return;
        }

        if self.stats.is_empty() {
            let empty_text = Paragraph::new("No proxy statistics available")
                .style(Style::default().fg(Color::Yellow))
                .block(Block::default().borders(Borders::ALL).title("Proxies"));
            f.render_widget(empty_text, area);
            return;
        }

        // Calculate uptime for each proxy
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let rows: Vec<Row> = self
            .stats
            .iter()
            .map(|stat| {
                let uptime_secs = now.saturating_sub(stat.start_time);
                let uptime = format_duration(uptime_secs);
                let bytes_sent = format_bytes(stat.bytes_sent);
                let bytes_received = format_bytes(stat.bytes_received);

                Row::new(vec![
                    Cell::from(stat.name.clone()),
                    Cell::from(format!("{}:{}", stat.publish_addr, stat.publish_port)),
                    Cell::from(stat.local_port.to_string()),
                    Cell::from(stat.active_connections.to_string()).style(Style::default().fg(
                        if stat.active_connections > 0 {
                            Color::Green
                        } else {
                            Color::Gray
                        },
                    )),
                    Cell::from(stat.total_connections.to_string()),
                    Cell::from(bytes_sent),
                    Cell::from(bytes_received),
                    Cell::from(uptime),
                ])
            })
            .collect();

        let header = Row::new(vec![
            Cell::from("Name").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Publish").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Local").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Active").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Total").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Sent").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Received").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("Uptime").style(Style::default().add_modifier(Modifier::BOLD)),
        ])
        .style(Style::default().bg(Color::DarkGray));

        let table = Table::new(
            rows,
            [
                Constraint::Length(15), // Name
                Constraint::Length(20), // Publish
                Constraint::Length(8),  // Local
                Constraint::Length(8),  // Active
                Constraint::Length(8),  // Total
                Constraint::Length(12), // Sent
                Constraint::Length(12), // Received
                Constraint::Length(12), // Uptime
            ],
        )
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Proxies ({})", self.stats.len())),
        )
        .style(Style::default().fg(Color::White));

        f.render_widget(table, area);
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let footer_text = vec![Line::from(vec![
            Span::styled("Press ", Style::default().fg(Color::Gray)),
            Span::styled(
                "'q'",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to quit, ", Style::default().fg(Color::Gray)),
            Span::styled(
                "'r'",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to refresh, ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("Auto-refresh: {}s", self.interval.as_secs()),
                Style::default().fg(Color::Cyan),
            ),
        ])];

        let footer = Paragraph::new(footer_text)
            .block(Block::default().borders(Borders::ALL).title("Controls"));

        f.render_widget(footer, area);
    }
}

/// Format bytes into human-readable format
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.2} {}", size, UNITS[unit_idx])
    }
}

/// Format duration into human-readable format
fn format_duration(seconds: u64) -> String {
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if days > 0 {
        format!("{}d {:02}h", days, hours)
    } else if hours > 0 {
        format!("{}h {:02}m", hours, minutes)
    } else if minutes > 0 {
        format!("{}m {:02}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}

/// Run the statistics dashboard
pub async fn run_dashboard(url: String, interval: u64) -> Result<()> {
    // Setup terminal
    enable_raw_mode().context("Failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("Failed to setup terminal")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("Failed to create terminal")?;

    let mut dashboard = Dashboard::new(url, interval);

    // Fetch initial data
    if let Err(e) = dashboard.fetch_stats().await {
        dashboard.error_message = Some(e.to_string());
    }

    let tick_rate = Duration::from_millis(250);
    let mut last_tick = SystemTime::now();
    let mut last_fetch = SystemTime::now();

    let result: Result<()> = loop {
        // Draw UI
        terminal
            .draw(|f| dashboard.render(f))
            .context("Failed to draw terminal")?;

        // Handle input events
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed().unwrap_or_default())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout).context("Event poll failed")? {
            if let Event::Key(key) = event::read().context("Failed to read event")? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        break Ok(());
                    }
                    KeyCode::Char('r') => {
                        // Manual refresh
                        if let Err(e) = dashboard.fetch_stats().await {
                            dashboard.error_message = Some(e.to_string());
                        }
                        last_fetch = SystemTime::now();
                    }
                    _ => {}
                }
            }
        }

        if last_tick.elapsed().unwrap_or_default() >= tick_rate {
            last_tick = SystemTime::now();

            // Auto-refresh
            if last_fetch.elapsed().unwrap_or_default() >= dashboard.interval {
                if let Err(e) = dashboard.fetch_stats().await {
                    dashboard.error_message = Some(e.to_string());
                }
                last_fetch = SystemTime::now();
            }
        }
    };

    // Restore terminal
    disable_raw_mode().context("Failed to disable raw mode")?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .context("Failed to restore terminal")?;
    terminal.show_cursor().context("Failed to show cursor")?;

    result
}
