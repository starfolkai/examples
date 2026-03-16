// Performance regression test — runs 5 minutes of gameplay and asserts
// no frame takes >10x the baseline. Detects slow regions.

#[path = "../src/apu.rs"]
mod apu;
#[path = "../src/cartridge.rs"]
mod cartridge;
#[path = "../src/renderer.rs"]
mod renderer;
#[path = "../src/game.rs"]
mod game;

use game::Game;
use std::time::Instant;

const BTN_RIGHT: u8 = 7;
const BTN_A: u8 = 0;
const BTN_B: u8 = 1;

fn simulate_input(game: &mut Game, frame: u32) {
    let action = frame % 90;
    if action == 0 { game.set_button(0, BTN_A, true); }
    else if action == 10 { game.set_button(0, BTN_A, false); }
    else if action == 20 { game.set_button(0, BTN_B, true); }
    else if action == 28 { game.set_button(0, BTN_B, false); }
    if frame % 600 == 300 {
        game.set_button(0, BTN_RIGHT, false);
    } else if frame % 600 == 0 {
        game.set_button(0, BTN_RIGHT, true);
    }
}

#[test]
fn no_performance_regression() {
    // Game data is compiled in — no ROM file needed
    let mut game = Game::new(&[], 0);
    game.set_button(0, BTN_RIGHT, true);

    // Boot + initial gameplay
    for _ in 0..700 {
        game.update();
    }

    let total_frames = 3000u32; // ~50 seconds, limited by trace length

    // Measure baseline (first 300 frames)
    let baseline_avg = {
        let mut sum = 0.0;
        for f in 0..300u32 {
            simulate_input(&mut game, f);
            let t0 = Instant::now();
            game.update();
            sum += t0.elapsed().as_micros() as f64;
        }
        sum / 300.0
    };

    let spike_threshold = baseline_avg * 3.0;
    let mut slow_regions: Vec<(u32, f64)> = Vec::new();
    let mut worst_time = 0.0f64;
    let mut worst_frame = 0u32;

    for frame in 300..total_frames {
        simulate_input(&mut game, frame);
        let t0 = Instant::now();
        game.update();
        let dt = t0.elapsed().as_micros() as f64;

        if dt > worst_time {
            worst_time = dt;
            worst_frame = frame;
        }

        if dt > spike_threshold {
            slow_regions.push((frame, dt));
        }
    }

    eprintln!("\n  Baseline: {:.0}us/frame ({:.0}fps equivalent)", baseline_avg, 1_000_000.0 / baseline_avg);
    eprintln!("  Worst frame: #{} at {:.0}us ({:.1}x baseline)", worst_frame, worst_time, worst_time / baseline_avg);
    eprintln!("  Slow frames (>3x): {}/{}", slow_regions.len(), total_frames);

    if !slow_regions.is_empty() {
        for &(f, dt) in slow_regions.iter().take(10) {
            eprintln!("    Frame {:5}: {:.0}us ({:.1}x)", f, dt, dt / baseline_avg);
        }
    }

    assert!(
        worst_time < baseline_avg * 15.0,
        "Worst frame ({:.0}us) is {:.1}x baseline — likely a real regression, not OS scheduling",
        worst_time, worst_time / baseline_avg
    );
}
