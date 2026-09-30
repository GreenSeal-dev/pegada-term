//! `pegada-term watch`: full-screen live power meter.

use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, Chart, Dataset, GraphType, Paragraph};
use ratatui::Frame;

use crate::state::State;
use crate::{daemon, fmt, paths, sessions};

const POINTS: usize = 240;

#[derive(Default)]
struct Meter {
    /// (seconds since the first sample, watts)
    points: VecDeque<(f64, f64)>,
    t0_ms: u64,
    last_seq: u64,
    state: Option<State>,
}

impl Meter {
    fn update(&mut self) {
        let Some(s) = State::read(&paths::state_file()) else {
            return;
        };
        if self.points.is_empty() {
            // Seed with what the sampler already knows.
            self.t0_ms = s
                .t_ms
                .saturating_sub(s.history[0].len() as u64 * s.interval_ms);
            for (i, p) in s.history[0].iter().enumerate().rev() {
                let t = s.t_ms.saturating_sub(i as u64 * s.interval_ms);
                self.points.push_back(self.at(t, u64::from(*p)));
            }
        } else if s.seq != self.last_seq {
            self.points.push_back(self.at(s.t_ms, s.power_mw));
        }
        self.points
            .drain(..self.points.len().saturating_sub(POINTS));
        self.last_seq = s.seq;
        self.state = Some(s);
    }

    fn at(&self, t_ms: u64, mw: u64) -> (f64, f64) {
        (
            t_ms.saturating_sub(self.t0_ms) as f64 / 1000.0,
            mw as f64 / 1000.0,
        )
    }
}

fn draw(frame: &mut Frame, meter: &Meter) {
    let [head, body, foot] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let dim = Style::default().add_modifier(Modifier::DIM);

    let Some(state) = meter.state.as_ref().filter(|s| s.is_fresh(paths::now_ms())) else {
        frame.render_widget(
            Paragraph::new("Waiting for the sampler… (if this stays, run `pegada-term doctor`)")
                .block(Block::bordered().title(" pegada-term ")),
            frame.area(),
        );
        return;
    };

    let watts: Vec<f64> = meter.points.iter().map(|p| p.1).collect();
    let avg = watts.iter().sum::<f64>() / watts.len().max(1) as f64;
    let peak = watts.iter().cloned().fold(0.0, f64::max);
    let idle = state.idle_mw as f64 / 1000.0;
    let stat = |label: &'static str, value: String, color: Color| {
        [
            Span::styled(format!("  {label} "), dim),
            Span::styled(
                value,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]
    };
    let mut spans = Vec::new();
    spans.extend(stat("now", fmt::power(state.power_mw), Color::Yellow));
    spans.extend(stat("avg", format!("{avg:.1} W"), Color::White));
    spans.extend(stat("peak", format!("{peak:.1} W"), Color::Red));
    spans.extend(stat("idle", fmt::power(state.idle_mw), Color::Green));
    spans.extend(stat(
        "above idle",
        fmt::power(state.power_mw.saturating_sub(state.idle_mw)),
        Color::Yellow,
    ));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).block(Block::bordered().title(" pegada-term watch ")),
        head,
    );

    let points: Vec<(f64, f64)> = meter.points.iter().copied().collect();
    let x0 = points.first().map_or(0.0, |p| p.0);
    let x1 = points.last().map_or(1.0, |p| p.0).max(x0 + 1.0);
    let top = (peak.max(idle) * 1.15).max(1.0);
    let idle_line = [(x0, idle), (x1, idle)];
    let chart = Chart::new(vec![
        Dataset::default()
            .name("idle")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Green))
            .data(&idle_line),
        Dataset::default()
            .name("power")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Yellow))
            .data(&points),
    ])
    .block(Block::bordered().title(format!(
        " whole-machine power, last {} ",
        fmt::duration(((x1 - x0) * 1000.0) as u64)
    )))
    .x_axis(Axis::default().bounds([x0, x1]))
    .y_axis(Axis::default().style(dim).bounds([0.0, top]).labels([
        "0 W".to_string(),
        format!("{:.0} W", top / 2.0),
        format!("{top:.0} W"),
    ]));
    frame.render_widget(chart, body);

    frame.render_widget(
        Paragraph::new(format!(
            " sensor {} · every {} ms · q to quit",
            state.source, state.interval_ms
        ))
        .style(dim),
        foot,
    );
}

pub fn run() -> i32 {
    // Registering as a session keeps the sampler alive while we watch.
    let session = match sessions::register_self() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("pegada-term: {e}");
            return 1;
        }
    };
    daemon::ensure_running();
    let mut terminal = ratatui::init();
    let mut meter = Meter::default();
    let result: io::Result<()> = (|| loop {
        meter.update();
        terminal.draw(|frame| draw(frame, &meter))?;
        let wait = meter
            .state
            .as_ref()
            .map_or(250, |s| s.interval_ms / 2)
            .max(50);
        if event::poll(Duration::from_millis(wait))? {
            if let Event::Key(key) = event::read()? {
                let ctrl_c =
                    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
                if ctrl_c || matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                    return Ok(());
                }
            }
        }
    })();
    ratatui::restore();
    let _ = std::fs::remove_file(session);
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("pegada-term: {e}");
            1
        }
    }
}
