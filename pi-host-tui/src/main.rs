use std::sync::Arc;

use arc_swap::ArcSwap;
use clap::Parser;

use crate::app::{App, RenderSnapshot};
use crate::host_state::HostState;

mod agent_host;
mod app;
mod commands;
mod config;
mod directives;
#[cfg(all(test, unix))]
mod e2e_tests;
mod editor;
mod extension;
mod host_state;
#[cfg(test)]
#[cfg(not(feature = "replay"))]
mod input_tests;
mod llm;
#[cfg(any(feature = "record", feature = "replay"))]
mod llm_cassette;
#[cfg(feature = "record")]
mod llm_record;
#[cfg(feature = "replay")]
mod llm_replay;
mod markdown;
mod model_picker;
mod onboarding;
#[cfg(test)]
mod onboarding_test;
#[cfg(all(test, unix))]
mod record_replay_test;
#[cfg(all(test, unix))]
mod replay_e2e_test;
mod scroll;
mod session;
mod session_log;
mod smoke_test;
mod theme;
mod tools;
#[cfg(test)]
mod tools_test;
// ---------------------------------------------------------------------------
// Pure render from snapshot (no App borrow — safe for ArcSwap read)
// ---------------------------------------------------------------------------

fn render_from_snapshot(frame: &mut ratatui::Frame<'_>, snap: &RenderSnapshot) {
    use crate::app::ChatEntry;
    use ratatui::{
        layout::{Constraint, Layout},
        style::{Color, Modifier, Style},
        text::{Line as LineText, Span as TextSpan},
        widgets::*,
    };

    let [chat_area, input_area, status_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    // Build chat lines from snapshot entries
    let mut lines: Vec<LineText<'static>> = vec![LineText::raw("")];

    for entry in snap.entries.iter() {
        match entry {
            ChatEntry::User(text) => {
                lines.push(LineText::from(vec![
                    TextSpan::styled("▌ ", Style::default().fg(Color::Cyan)),
                    TextSpan::styled(
                        "You",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    TextSpan::raw(": "),
                ]));
                for l in text.lines() {
                    lines.push(LineText::from(vec![
                        TextSpan::styled("▌ ", Style::default().fg(Color::Cyan)),
                        TextSpan::styled(l.to_string(), Style::default().fg(Color::White)),
                    ]));
                }
                lines.push(LineText::raw(""));
            }
            ChatEntry::Assistant(text) => {
                for line in &text.lines {
                    let is_blank = line.spans.is_empty()
                        || line.spans.iter().all(|s| s.content.as_ref().is_empty());
                    if !is_blank {
                        let mut spans: Vec<TextSpan<'static>> =
                            vec![TextSpan::styled("▌ ", Style::default().fg(Color::Cyan))];
                        for span in &line.spans {
                            spans.push(TextSpan::styled(span.content.to_string(), span.style));
                        }
                        lines.push(LineText::from(spans));
                    } else {
                        lines.push(LineText::raw(""));
                    }
                }
                lines.push(LineText::raw(""));
            }
            ChatEntry::System(text) => {
                lines.push(LineText::from(vec![
                    TextSpan::styled("◇ ", Style::default().fg(Color::Rgb(80, 80, 80))),
                    TextSpan::styled(text.to_string(), Style::default().fg(Color::Gray)),
                ]));
                lines.push(LineText::raw(""));
            }
        }
    }

    // Streaming spinner indicator in chat area
    if snap.running {
        let elapsed_ms = snap
            .streaming_start
            .map(|s| s.elapsed().as_millis() as usize)
            .unwrap_or(0);
        const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "▓", "█", "▒", "░"];
        let spinner = FRAMES[(elapsed_ms / 120) % FRAMES.len()];
        lines.push(LineText::from(vec![
            TextSpan::styled(
                format!("  {} ", spinner),
                Style::default().fg(Color::Yellow),
            ),
            TextSpan::styled(
                "Thinking",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    // When suggestions are active, steal space from chat_area (not input_area)
    let (chat_render_area, sug_area) = if snap.show_suggestions && !snap.suggestions.is_empty() {
        let sug_height = snap.suggestions.len() as u16 + 1;
        let rects = Layout::vertical([Constraint::Fill(1), Constraint::Length(sug_height)])
            .split(chat_area);
        (rects[0], Some(rects[1]))
    } else {
        (chat_area, None)
    };

    frame.render_widget(paragraph, chat_render_area);

    if let Some(area) = sug_area {
        render_suggestions(frame, area, snap);
    }
    render_input_from_snapshot(frame, input_area, snap);

    // Model picker overlay
    if snap.show_model_picker {
        render_model_picker(frame, chat_area, snap);
    }

    // Status bar
    let status_line = LineText::from(vec![
        TextSpan::styled(
            " pio",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        TextSpan::raw(" · "),
        TextSpan::styled(&snap.model_name, Style::default().fg(Color::DarkGray)),
    ]);
    let status = Paragraph::new(status_line).style(
        Style::default()
            .bg(Color::Rgb(20, 20, 30))
            .fg(Color::DarkGray),
    );
    frame.render_widget(status, status_area);
}

/// Render command suggestions list above the input area.
fn render_suggestions(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    snap: &RenderSnapshot,
) {
    use ratatui::{
        style::{Color, Modifier, Style},
        text::{Line as LineText, Span as TextSpan},
        widgets::*,
    };

    let mut lines: Vec<LineText<'static>> = Vec::new();
    for (i, sug) in snap.suggestions.iter().enumerate() {
        let is_selected = snap.suggestion_selected == Some(i);
        let prefix = if is_selected { "▸ " } else { "  " };
        let style = if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(140, 140, 140))
        };
        lines.push(LineText::from(vec![
            TextSpan::styled(prefix.to_string(), style),
            TextSpan::styled(sug.to_string(), style),
        ]));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);
}

/// Render model picker overlay — centered in the chat area.
fn render_model_picker(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    snap: &RenderSnapshot,
) {
    use ratatui::{
        layout::{Constraint, Layout},
        style::{Color, Modifier, Style},
        text::{Line as LineText, Span as TextSpan},
        widgets::*,
    };

    let picker_height =
        (snap.model_picker_items.len() as u16 + 3).min(area.height.saturating_sub(2));
    let picker_width = (area.width / 2).max(30).min(area.width.saturating_sub(2));

    // Center: split horizontally (Fill, picker, Fill), then vertically (Fill, picker, Fill)
    let center_x = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(picker_width),
        Constraint::Fill(1),
    ])
    .split(area)[1];
    let picker_area = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(picker_height),
        Constraint::Fill(1),
    ])
    .split(center_x)[1];

    let mut lines: Vec<LineText> = Vec::new();
    // Header
    lines.push(LineText::from(vec![TextSpan::styled(
        " Model Selection ",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )]));
    // Filter line
    lines.push(LineText::from(vec![
        TextSpan::styled("Filter: ", Style::default().fg(Color::Rgb(120, 120, 120))),
        TextSpan::styled(
            snap.model_picker_filter.clone(),
            Style::default()
                .fg(Color::Rgb(180, 180, 180))
                .add_modifier(Modifier::ITALIC),
        ),
    ]));
    lines.push(LineText::raw(""));
    // Model list
    for (i, model) in snap.model_picker_items.iter().enumerate() {
        let is_selected = i == snap.model_picker_selected;
        let prefix = if is_selected { "▸ " } else { "  " };
        let style = if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(140, 140, 140))
        };
        lines.push(LineText::from(vec![
            TextSpan::styled(prefix.to_string(), style),
            TextSpan::styled(model.clone(), style),
        ]));
    }
    // Footer
    lines.push(LineText::raw(""));
    lines.push(LineText::from(vec![TextSpan::styled(
        "Enter: select  Esc: cancel  ↑↓: navigate",
        Style::default().fg(Color::Rgb(100, 100, 100)),
    )]));

    let paragraph = Paragraph::new(lines).block(
        Block::new()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(60, 60, 90))),
    );
    frame.render_widget(paragraph, picker_area);
}

fn render_input_from_snapshot(
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    snap: &RenderSnapshot,
) {
    use ratatui::{
        style::{Color, Modifier, Style},
        text::{Line as LineText, Span as TextSpan},
        widgets::*,
    };

    let mut spans = vec![
        TextSpan::styled("▌ ", Style::default().fg(Color::Cyan)),
        TextSpan::raw(&snap.input_text),
    ];

    if snap.show_quit_prompt {
        spans.push(TextSpan::styled(
            " Press Ctrl+C again to quit",
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::ITALIC),
        ));
    } else if snap.running {
        let elapsed_ms = snap
            .streaming_start
            .map(|s| s.elapsed().as_millis() as usize)
            .unwrap_or(0);
        const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "▓", "█", "▒", "░"];
        let spinner = FRAMES[(elapsed_ms / 120) % FRAMES.len()];
        spans.push(TextSpan::styled(
            format!(" {}", spinner),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
        ));
    }

    let input_text = ratatui::text::Text::from(LineText::from(spans));
    let paragraph = Paragraph::new(input_text).block(
        Block::new()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::Rgb(40, 40, 60))),
    );
    frame.render_widget(paragraph, area);

    // Cursor position: "▌ " prefix (2 chars) + cursor offset into text
    let input_width = unicode_width::UnicodeWidthStr::width(
        &snap.input_text[..snap.input_cursor_pos.min(snap.input_text.len())],
    ) as u16;
    frame.set_cursor_position(ratatui::layout::Position {
        x: area
            .x
            .saturating_add(2)
            .saturating_add(input_width)
            .min(area.x + area.width - 1),
        y: area.y + 1, // Block::borders(TOP) draws on row area.y
    });
}

// ---------------------------------------------------------------------------
// Async entry point — render task at 30fps + actor loop with inline input poll
// ---------------------------------------------------------------------------

async fn run_async(
    terminal: Arc<std::sync::Mutex<ratatui::DefaultTerminal>>,
    snapshot: Arc<ArcSwap<RenderSnapshot>>,
    app: App,
) {
    // Render task: 30fps reads from ArcSwap (never blocks)
    let term_clone = terminal.clone();
    let snap_clone = snapshot.clone();
    let render_handle = tokio::spawn(async move {
        use std::time::Duration as Td;
        let mut interval = tokio::time::interval(Td::from_millis(33));

        loop {
            interval.tick().await;
            let snap = snap_clone.load_full(); // lock-free read of Arc<RenderSnapshot>
            if let Ok(mut t) = term_clone.lock() {
                let _ = t.draw(|f| render_from_snapshot(f, &snap));
            }
        }
    });

    // Actor loop: owns App, polls crossterm events inline on blocking thread
    app::run_actor_loop(app).await;

    // Abort render task so it doesn't panic on drop (current_thread runtime)
    render_handle.abort();
}

// ---------------------------------------------------------------------------
// CLI + ONBOARDING (unchanged)
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "pio", about = "Terminal coding agent")]
struct Cli {
    /// Model ID (overrides config)
    #[arg(long, env = "PI_MODEL")]
    model: Option<String>,

    /// API base URL (overrides config)
    #[arg(long, env = "PI_BASE_URL")]
    base_url: Option<String>,

    /// Provider family
    #[arg(long, env = "PI_PROVIDER")]
    provider: Option<String>,

    /// API key (overrides config and env vars)
    #[arg(long, env = "PI_API_KEY")]
    api_key: Option<String>,

    /// System prompt (per-invocation, not persisted in config)
    #[arg(long)]
    system: Option<String>,

    /// Session ID for persistent conversation history
    #[arg(short, long, env = "PI_SESSION_ID")]
    session_id: Option<String>,

    /// Record LLM calls to a cassette file (requires `record` feature)
    #[cfg(feature = "record")]
    #[arg(long)]
    record_to: Option<std::path::PathBuf>,

    /// Replay LLM calls from a cassette file (requires `replay` feature)
    #[cfg(feature = "replay")]
    #[arg(long)]
    replay_from: Option<std::path::PathBuf>,

    /// Skip the onboarding wizard
    #[arg(long)]
    skip_onboarding: bool,
}

fn should_run_onboarding(cli: &Cli) -> bool {
    if cli.model.is_some()
        || cli.provider.is_some()
        || cli.api_key.is_some()
        || cli.base_url.is_some()
    {
        return false;
    }
    if config::project_config_path().exists() || config::global_config_path().exists() {
        return false;
    }
    let api_key_envs = [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "DEEPSEEK_API_KEY",
        "PI_API_KEY",
    ];
    for key in &api_key_envs {
        if std::env::var(key).ok().is_some_and(|v| !v.is_empty()) {
            return false;
        }
    }
    true
}

fn main() -> Result<(), app::TuiError> {
    let log_dir = config::home_dir().join(".pi-oxide").join("logs");
    std::fs::create_dir_all(&log_dir).ok();
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("pio.log"))
        .expect("failed to open log file");

    tracing_subscriber::fmt()
        .with_writer(log_file)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();

    // Run onboarding if no config exists and no API key is available
    let resolved = if !cli.skip_onboarding && should_run_onboarding(&cli) {
        if let Some(onboard) = onboarding::run() {
            config::resolve(
                Some(&onboard.model),
                Some(&onboard.provider),
                Some(&onboard.api_key),
                Some(&onboard.base_url),
            )
        } else {
            config::resolve(
                cli.model.as_deref(),
                cli.provider.as_deref(),
                cli.api_key.as_deref(),
                cli.base_url.as_deref(),
            )
        }
    } else {
        config::resolve(
            cli.model.as_deref(),
            cli.provider.as_deref(),
            cli.api_key.as_deref(),
            cli.base_url.as_deref(),
        )
    };

    let system_prompt = cli
        .system
        .clone()
        .unwrap_or_else(|| "You are a helpful coding assistant.".into());

    let wire_format = match resolved.provider.as_str() {
        "anthropic" | "anthropic-compat" | "deepseek-anthropic" => {
            crate::llm::WireFormat::Anthropic
        }
        "openai" | "openai-compat" | "deepseek" => crate::llm::WireFormat::OpenAI,
        other => {
            eprintln!("Unknown provider: {other}. Supported: anthropic, openai, deepseek");
            std::process::exit(1);
        }
    };

    let session_backend = session::FileSystemSessionBackend::new();
    let (host_state, session_ctx) = if let Some(ref id) = cli.session_id {
        if let Some(data) = session_backend.load(id) {
            let (hs, ctx) = HostState::restore(data);
            (Some(hs), Some(ctx))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    let cwd = std::env::current_dir()?;
    let terminal: Arc<std::sync::Mutex<ratatui::DefaultTerminal>> =
        Arc::new(std::sync::Mutex::new(ratatui::init()));
    print!("\x1b[2 q"); // steady block cursor

    let app = App::new(
        &system_prompt,
        &resolved.model,
        &resolved.api_key,
        &resolved.base_url,
        cli.session_id.clone(),
        host_state,
        session_ctx,
        &cwd,
        wire_format,
        &resolved.provider,
        #[cfg(feature = "record")]
        cli.record_to,
        #[cfg(feature = "replay")]
        cli.replay_from,
        resolved.clone(),
    )?;

    // Publish initial snapshot before starting tasks
    app.publish_snapshot();

    // Clone the shared ArcSwap so render task and actor share the same instance.
    let snapshot = app.snapshot.clone();

    // Block on async runtime — runs actor + render tasks concurrently
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(run_async(terminal, snapshot, app));

    ratatui::restore();
    print!("\x1b[0 q");
    Ok(())
}
