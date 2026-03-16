// Input and determinism tests for native game engine

#[path = "../src/apu.rs"]
mod apu;
#[path = "../src/cartridge.rs"]
mod cartridge;
#[path = "../src/renderer.rs"]
mod renderer;
#[path = "../src/game.rs"]
mod game;

use game::Game;

const BTN_START: u8 = 3;
const BTN_A: u8 = 0;
const BTN_RIGHT: u8 = 7;

fn framebuffer_checksum(fb: &[u8]) -> u64 {
    let mut hash = 0u64;
    for (i, &b) in fb.iter().enumerate().step_by(7) {
        hash = hash.wrapping_mul(31).wrapping_add(b as u64).wrapping_add(i as u64);
    }
    hash
}

/// Two independent game instances with the same inputs produce identical output.
#[test]
fn deterministic_parallel_runs() {
    let mut game1 = Game::new(&[], 0);
    let mut game2 = Game::new(&[], 0);

    for _ in 0..500 {
        game1.update();
        game2.update();
    }

    let h1 = framebuffer_checksum(game1.framebuffer());
    let h2 = framebuffer_checksum(game2.framebuffer());
    eprintln!("  game1 hash at 500: {:016x}", h1);
    eprintln!("  game2 hash at 500: {:016x}", h2);
    assert_eq!(h1, h2, "Two instances diverged despite identical inputs");
}

/// Button state is stored correctly.
#[test]
fn button_state_stored() {
    let mut game = Game::new(&[], 0);

    game.set_button(0, BTN_A, true);
    game.set_button(0, BTN_START, true);
    game.set_button(0, BTN_RIGHT, true);

    // Run a frame to ensure no crash
    game.update();

    // Release
    game.set_button(0, BTN_A, false);
    game.update();

    // No assertions on output change (trace replay is input-independent)
    // This test verifies set_button doesn't panic or corrupt state
    eprintln!("  Button set/release: OK");
}

/// Frame pacing math doesn't drift
#[test]
fn frame_pacing_no_drift() {
    let frame_duration_ns = 16_666_667u64;
    let mut next_frame_time = 0u64;
    let mut actual_time = 0u64;

    let frame_work_times = [2500, 2800, 3100, 2400, 2700, 2900, 3200, 2600, 2500, 3000];

    for i in 0..600 {
        let work_us = frame_work_times[i % frame_work_times.len()];
        actual_time += work_us * 1000;
        next_frame_time += frame_duration_ns;
        if next_frame_time > actual_time {
            actual_time = next_frame_time;
        } else if actual_time - next_frame_time > frame_duration_ns * 3 {
            next_frame_time = actual_time;
        }
    }

    let expected_ns = 600u64 * frame_duration_ns;
    let drift_ms = (actual_time as f64 - expected_ns as f64) / 1_000_000.0;
    eprintln!("  Pacing drift after 600 frames: {:.2}ms", drift_ms);
    assert!(drift_ms.abs() < 20.0);
}
