//! Floating waveform fluid ribbon visualizer.
//!
//! Renders multi-strand harmonic fluid wave ribbons oscillating around a horizontal
//! center axis. Features smooth window envelope tapering at both ends, live audio
//! waveform modulation, bass hydraulic expansions, transient shimmers, and Braille
//! sub-cell precision. When silent, weaves organic harmonic ribbons like a luminous
//! soundwave.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use myx::audio::visualizer::AudioWaveSnapshot;
use myx::gradient::lerp_color;
use myx::theme::Theme;

/// Multi-strand fluid waveform dynamics state.
#[derive(Debug, Clone)]
pub struct FluidSimulation {
    /// Elapsed simulation time in seconds.
    pub time: f32,
    /// User sensitivity multiplier.
    pub sensitivity: f32,
    /// Smoothed RMS volume level.
    pub smoothed_rms: f32,
    /// Smoothed bass energy level.
    pub smoothed_bass: f32,
    /// Smoothed treble / transient energy level.
    pub smoothed_treble: f32,
    /// Sliding normalized waveform samples.
    pub waveform: Vec<f32>,
    /// Whether audio is actively playing above the gate floor.
    pub is_active: bool,
    /// Particle / column resolution.
    pub num_columns: usize,
}

impl Default for FluidSimulation {
    fn default() -> Self {
        Self::new(128)
    }
}

impl FluidSimulation {
    /// Create a new fluid simulation with the given column count.
    pub fn new(num_columns: usize) -> Self {
        Self {
            time: 0.0,
            sensitivity: 1.0,
            smoothed_rms: 0.0,
            smoothed_bass: 0.0,
            smoothed_treble: 0.0,
            waveform: vec![0.0; 128],
            is_active: false,
            num_columns: num_columns.max(32),
        }
    }

    /// Resize column count dynamically on terminal resize.
    pub fn resize(&mut self, new_size: usize) {
        self.num_columns = new_size.max(32);
    }

    /// Inject audio waveform and band energy into the fluid simulation.
    pub fn inject_waveform(&mut self, snapshot: &AudioWaveSnapshot, sensitivity: f32) {
        self.sensitivity = sensitivity.clamp(0.2, 4.0);
        self.is_active = snapshot.is_active;

        if snapshot.is_active {
            // Smooth attack / release for RMS and band energies
            let attack = 0.60;
            let release = 0.16;

            self.smoothed_rms = if snapshot.rms > self.smoothed_rms {
                self.smoothed_rms + (snapshot.rms - self.smoothed_rms) * attack
            } else {
                self.smoothed_rms + (snapshot.rms - self.smoothed_rms) * release
            };

            self.smoothed_bass = if snapshot.bass_energy > self.smoothed_bass {
                self.smoothed_bass + (snapshot.bass_energy - self.smoothed_bass) * attack
            } else {
                self.smoothed_bass + (snapshot.bass_energy - self.smoothed_bass) * release
            };

            self.smoothed_treble = if snapshot.treble_energy > self.smoothed_treble {
                self.smoothed_treble + (snapshot.treble_energy - self.smoothed_treble) * attack
            } else {
                self.smoothed_treble + (snapshot.treble_energy - self.smoothed_treble) * release
            };

            // Ingest normalized waveform
            if !snapshot.waveform.is_empty() {
                if self.waveform.len() != snapshot.waveform.len() {
                    self.waveform = snapshot.waveform.clone();
                } else {
                    for (dst, &src) in self.waveform.iter_mut().zip(&snapshot.waveform) {
                        *dst += (src - *dst) * 0.55;
                    }
                }
            }
        } else {
            // Decay energies smoothly towards idle
            self.smoothed_rms *= 0.88;
            self.smoothed_bass *= 0.88;
            self.smoothed_treble *= 0.88;
            for w in &mut self.waveform {
                *w *= 0.85;
            }
        }
    }

    /// Advance simulation clock.
    pub fn step(&mut self, dt: f32) {
        self.time += dt;
    }
}

/// Convert dot coordinates (0..1 col, 0..3 row) to standard Unicode Braille bitmask.
#[inline]
fn braille_bit(dot_x: usize, dot_y: usize) -> u8 {
    match (dot_x, dot_y) {
        (0, 0) => 0x01, // Dot 1
        (0, 1) => 0x02, // Dot 2
        (0, 2) => 0x04, // Dot 3
        (0, 3) => 0x40, // Dot 7
        (1, 0) => 0x08, // Dot 4
        (1, 1) => 0x10, // Dot 5
        (1, 2) => 0x20, // Dot 6
        (1, 3) => 0x80, // Dot 8
        _ => 0,
    }
}

/// Render the floating waveform fluid visualizer into `area`.
pub fn render_fluid_visualizer(
    frame: &mut Frame<'_>,
    area: Rect,
    sim: &FluidSimulation,
    theme: Theme,
) {
    if area.width < 4 || area.height < 2 {
        return;
    }

    let cols = area.width as usize;
    let rows = area.height as usize;
    let sub_w = cols * 2;
    let sub_h = rows * 4;

    let center_y = (sub_h as f32 - 1.0) / 2.0;
    let max_amp = (center_y * 0.90).max(1.0);

    let sens = sim.sensitivity.clamp(0.2, 3.5);
    let active = sim.is_active;

    let rms = sim.smoothed_rms;
    let bass = sim.smoothed_bass;
    let treble = sim.smoothed_treble;
    let t = sim.time;

    // Amplitude scale: surges with live audio, gentle ribbon when silent
    let amp_scale = if active {
        (0.40 + 0.60 * rms + 0.50 * bass) * sens
    } else {
        0.34
    };

    let speed = if active {
        3.2 + 3.8 * treble
    } else {
        1.7
    };

    let wave_len = sim.waveform.len().max(1);

    // Precalculate sub-column heights for strands
    let mut y1_arr = Vec::with_capacity(sub_w);
    let mut y2_arr = Vec::with_capacity(sub_w);
    let mut y3_arr = Vec::with_capacity(sub_w);
    let mut env_arr = Vec::with_capacity(sub_w);

    let pi = std::f32::consts::PI;

    for x in 0..sub_w {
        let u = x as f32 / (sub_w - 1).max(1) as f32;
        // Window envelope: smooth sine bell tapering to 0 at left and right edges
        let env = (pi * u).sin().powf(1.30);
        env_arr.push(env);

        let w_idx = ((u * wave_len as f32) as usize).min(wave_len - 1);
        let raw_w = if active {
            sim.waveform.get(w_idx).copied().unwrap_or(0.0)
        } else {
            0.0
        };
        let live_w = raw_w * sens * 0.70;

        // Strand 1 (Primary Cyan/Accent Wave)
        let wave1_phase = speed * t + 3.0 * pi * u;
        let wave1_disp = (live_w + 0.44 * wave1_phase.sin() + 0.18 * (wave1_phase * 1.9 - 0.6).sin())
            * amp_scale
            * max_amp;
        let y1 = (center_y - env * wave1_disp).clamp(0.0, (sub_h - 1) as f32);

        // Strand 2 (Complementary Magenta/Primary Ribbon)
        let wave2_phase = speed * t * 1.08 + 3.2 * pi * u + 2.15;
        let wave2_disp = (-live_w * 0.85 + 0.46 * wave2_phase.sin() + 0.18 * (wave2_phase * 2.1).cos())
            * amp_scale
            * max_amp;
        let y2 = (center_y + env * wave2_disp).clamp(0.0, (sub_h - 1) as f32);

        // Strand 3 (Harmonic Shimmer Ribbon)
        let wave3_phase = speed * t * 1.55 + 5.2 * pi * u;
        let wave3_disp = (0.26 * wave3_phase.sin() + live_w * 0.35 * (4.0 * pi * u).cos())
            * amp_scale
            * max_amp;
        let y3 = (center_y - env * wave3_disp).clamp(0.0, (sub_h - 1) as f32);

        y1_arr.push(y1);
        y2_arr.push(y2);
        y3_arr.push(y3);
    }

    // Build terminal lines
    let mut lines = Vec::with_capacity(rows);

    for r in 0..rows {
        let mut spans = Vec::with_capacity(cols);

        for c in 0..cols {
            let sx0 = c * 2;
            let mut mask1 = 0u8;
            let mut mask2 = 0u8;
            let mut mask3 = 0u8;
            let mut axis_mask = 0u8;
            let mut thread_mask = 0u8;

            for dot_x in 0..2 {
                let sx = sx0 + dot_x;
                let y1 = y1_arr[sx];
                let y2 = y2_arr[sx];
                let y3 = y3_arr[sx];
                let env = env_arr[sx];

                let y_top = y1.min(y2);
                let y_bot = y1.max(y2);

                for dot_y in 0..4 {
                    let sy = (r * 4 + dot_y) as f32;

                    // Strand 1 dots
                    if (sy - y1).abs() <= 0.62 {
                        mask1 |= braille_bit(dot_x, dot_y);
                    }
                    // Strand 2 dots
                    if (sy - y2).abs() <= 0.62 {
                        mask2 |= braille_bit(dot_x, dot_y);
                    }
                    // Strand 3 dots
                    if (sy - y3).abs() <= 0.62 {
                        mask3 |= braille_bit(dot_x, dot_y);
                    }
                    // Center axis dots near edges
                    if env < 0.12 && (sy - center_y).abs() <= 0.55 {
                        axis_mask |= braille_bit(dot_x, dot_y);
                    }
                    // Vertical thread connecting dots between primary and counter waves
                    if (y_bot - y_top) > 2.8
                        && sx % 3 == 0
                        && sy > y_top + 0.8
                        && sy < y_bot - 0.8
                        && (sy as usize).is_multiple_of(2)
                    {
                        thread_mask |= braille_bit(dot_x, dot_y);
                    }
                }
            }

            // Render dot priority and colors
            if mask1 > 0 && mask2 > 0 {
                // Crossing node: luminous white/core glow with bold
                let combined = mask1 | mask2;
                let ch = char::from_u32(0x2800 + combined as u32).unwrap_or(' ');
                spans.push(Span::styled(
                    ch.to_string(),
                    Style::default()
                        .fg(theme.text.into())
                        .add_modifier(Modifier::BOLD),
                ));
            } else if mask1 > 0 {
                // Strand 1: electric cyan / accent
                let ch = char::from_u32(0x2800 + mask1 as u32).unwrap_or(' ');
                spans.push(Span::styled(
                    ch.to_string(),
                    Style::default()
                        .fg(theme.accent.into())
                        .add_modifier(Modifier::BOLD),
                ));
            } else if mask2 > 0 {
                // Strand 2: magenta / primary
                let ch = char::from_u32(0x2800 + mask2 as u32).unwrap_or(' ');
                spans.push(Span::styled(
                    ch.to_string(),
                    Style::default()
                        .fg(theme.primary.into())
                        .add_modifier(Modifier::BOLD),
                ));
            } else if mask3 > 0 {
                // Strand 3: soft violet / secondary
                let ch = char::from_u32(0x2800 + mask3 as u32).unwrap_or(' ');
                spans.push(Span::styled(
                    ch.to_string(),
                    Style::default().fg(theme.secondary.into()),
                ));
            } else if thread_mask > 0 {
                // Vertical laser threads connecting waves
                let ch = char::from_u32(0x2800 + thread_mask as u32).unwrap_or(' ');
                let thread_color = lerp_color(theme.primary, theme.border_dimmest, 0.55);
                spans.push(Span::styled(
                    ch.to_string(),
                    Style::default().fg(thread_color.into()),
                ));
            } else if axis_mask > 0 {
                // Center horizontal axis
                let ch = char::from_u32(0x2800 + axis_mask as u32).unwrap_or(' ');
                spans.push(Span::styled(
                    ch.to_string(),
                    Style::default().fg(theme.border_subtle.into()),
                ));
            } else {
                spans.push(Span::raw(" "));
            }
        }

        lines.push(Line::from(spans));
    }

    let widget = Paragraph::new(lines).alignment(Alignment::Center);
    frame.render_widget(widget, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fluid_simulation_step_and_stability() {
        let mut sim = FluidSimulation::new(32);
        assert_eq!(sim.num_columns, 32);

        // Idle step
        let snap = AudioWaveSnapshot::default();
        sim.inject_waveform(&snap, 1.0);
        sim.step(0.033);

        assert!(!sim.time.is_nan());
        assert_eq!(sim.smoothed_bass, 0.0);

        // Inject active wave impulse
        let active = AudioWaveSnapshot {
            is_active: true,
            waveform: vec![0.5; 128],
            bass_energy: 0.8,
            treble_energy: 0.4,
            rms: 0.6,
            ..Default::default()
        };
        sim.inject_waveform(&active, 1.2);

        for _ in 0..10 {
            sim.step(0.033);
        }

        assert!(sim.smoothed_bass > 0.1);
        assert!(sim.smoothed_rms > 0.1);
    }

    #[test]
    fn braille_bit_coverage() {
        assert_eq!(braille_bit(0, 0), 0x01);
        assert_eq!(braille_bit(0, 1), 0x02);
        assert_eq!(braille_bit(0, 2), 0x04);
        assert_eq!(braille_bit(0, 3), 0x40);
        assert_eq!(braille_bit(1, 0), 0x08);
        assert_eq!(braille_bit(1, 1), 0x10);
        assert_eq!(braille_bit(1, 2), 0x20);
        assert_eq!(braille_bit(1, 3), 0x80);
    }

    #[test]
    fn render_fluid_visualizer_draws_without_panic() {
        use myx::theme::TOKYONIGHT;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let sim = FluidSimulation::new(64);

        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 40, 8);
                render_fluid_visualizer(f, area, &sim, TOKYONIGHT);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.area.width, 40);
        assert_eq!(buffer.area.height, 10);
    }
}
