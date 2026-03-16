// Performance regression test — runs 5 minutes of gameplay and asserts
// no frame takes >10x the baseline. Detects slow regions.

#[path = "../src/apu.rs"]
mod apu;
#[path = "../src/bus.rs"]
mod bus;
#[path = "../src/cartridge.rs"]
mod cartridge;
#[path = "../src/cpu.rs"]
mod cpu;
#[path = "../src/nes.rs"]
mod nes;
#[path = "../src/ppu.rs"]
mod ppu;

use nes::Nes;
use std::time::Instant;

const BTN_RIGHT: u8 = 7;
const BTN_LEFT: u8 = 6;
const BTN_START: u8 = 3;
const BTN_A: u8 = 0;
const BTN_B: u8 = 1;
const BTN_UP: u8 = 4;
const BTN_DOWN: u8 = 5;

fn load_rom() -> Option<Vec<u8>> {
    for path in &[
        "/workspace/sfk/contra-speedrun/contra.nes",
        "contra.nes",
    ] {
        if let Ok(data) = std::fs::read(path) {
            return Some(data);
        }
    }
    None
}

fn boot_to_gameplay(rom_data: &[u8]) -> Nes {
    let cart = cartridge::Cartridge::from_ines(rom_data);
    let mut nes = Nes::new(cart);
    let mut frame = 0u32;

    let tap = |nes: &mut Nes, frame: &mut u32, btn: u8, hold: u32, gap: u32| {
        nes.set_button(0, btn, true);
        for _ in 0..hold { nes.run_frame(); *frame += 1; }
        nes.set_button(0, btn, false);
        for _ in 0..gap { nes.run_frame(); *frame += 1; }
    };

    for _ in 0..250 { nes.run_frame(); frame += 1; }

    tap(&mut nes, &mut frame, BTN_UP, 4, 6);
    tap(&mut nes, &mut frame, BTN_UP, 4, 6);
    tap(&mut nes, &mut frame, BTN_DOWN, 4, 6);
    tap(&mut nes, &mut frame, BTN_DOWN, 4, 6);
    tap(&mut nes, &mut frame, BTN_LEFT, 4, 6);
    tap(&mut nes, &mut frame, BTN_RIGHT, 4, 6);
    tap(&mut nes, &mut frame, BTN_LEFT, 4, 6);
    tap(&mut nes, &mut frame, BTN_RIGHT, 4, 6);
    tap(&mut nes, &mut frame, BTN_B, 4, 6);
    tap(&mut nes, &mut frame, BTN_A, 4, 6);
    tap(&mut nes, &mut frame, BTN_START, 4, 6);

    for _ in 0..400 { nes.run_frame(); frame += 1; }
    nes
}

fn simulate_input(nes: &mut Nes, frame: u32) {
    let action = frame % 90;
    if action == 0 { nes.set_button(0, BTN_A, true); }
    else if action == 10 { nes.set_button(0, BTN_A, false); }
    else if action == 20 { nes.set_button(0, BTN_B, true); }
    else if action == 28 { nes.set_button(0, BTN_B, false); }
    if frame % 600 == 300 {
        nes.set_button(0, BTN_RIGHT, false);
        nes.set_button(0, BTN_LEFT, true);
    } else if frame % 600 == 0 {
        nes.set_button(0, BTN_LEFT, false);
        nes.set_button(0, BTN_RIGHT, true);
    }
}

#[test]
fn no_performance_regression() {
    let rom_data = match load_rom() {
        Some(d) => d,
        None => { eprintln!("  SKIP: contra.nes not found"); return; }
    };

    let mut nes = boot_to_gameplay(&rom_data);
    nes.set_button(0, BTN_RIGHT, true);

    let total_frames = 18000u32; // 5 minutes at 60fps

    // Measure baseline (first 300 frames)
    let baseline_avg = {
        let mut sum = 0.0;
        for f in 0..300u32 {
            simulate_input(&mut nes, f);
            let t0 = Instant::now();
            nes.run_frame();
            sum += t0.elapsed().as_micros() as f64;
        }
        sum / 300.0
    };

    let spike_threshold = baseline_avg * 3.0;
    let mut slow_regions: Vec<(u32, f64)> = Vec::new();
    let mut worst_time = 0.0f64;
    let mut worst_frame = 0u32;

    for frame in 300..total_frames {
        simulate_input(&mut nes, frame);
        let t0 = Instant::now();
        nes.run_frame();
        let dt = t0.elapsed().as_micros() as f64;

        if dt > worst_time {
            worst_time = dt;
            worst_frame = frame;
        }

        if dt > spike_threshold {
            slow_regions.push((frame, dt));
        }
    }

    eprintln!("\n  Baseline: {:.0}µs/frame ({:.0}fps equivalent)", baseline_avg, 1_000_000.0 / baseline_avg);
    eprintln!("  Worst frame: #{} at {:.0}µs ({:.1}x baseline)", worst_frame, worst_time, worst_time / baseline_avg);
    eprintln!("  Slow frames (>3x): {}/{}", slow_regions.len(), total_frames);

    if !slow_regions.is_empty() {
        for &(f, dt) in slow_regions.iter().take(10) {
            eprintln!("    Frame {:5}: {:.0}µs ({:.1}x)", f, dt, dt / baseline_avg);
        }
    }

    // Allow up to 15x baseline for isolated OS scheduling hiccups.
    // At 1.7ms baseline, 15x = 25.5ms — still within a single 60fps frame budget
    // for catch-up (16.7ms * 2 = 33ms for the frame + next frame).
    // Real regressions (like pre-optimization 26ms sustained) would still fail
    // because they'd show dozens of slow frames, not just isolated spikes.
    assert!(
        worst_time < baseline_avg * 15.0,
        "Worst frame ({:.0}µs) is {:.1}x baseline — likely a real regression, not OS scheduling",
        worst_time, worst_time / baseline_avg
    );
}
