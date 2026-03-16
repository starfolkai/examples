// Performance regression test — profiles exact bottleneck causing lag
// at specific game points (level transitions, heavy sprite scenes).

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

    let mut tap = |nes: &mut Nes, frame: &mut u32, btn: u8, hold: u32, gap: u32| {
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

/// Main test: run 20k frames, profile every frame, find the exact bottleneck
#[test]
fn profile_gameplay_frames() {
    let rom_data = match load_rom() {
        Some(d) => d,
        None => { eprintln!("  SKIP: contra.nes not found"); return; }
    };

    let mut nes = boot_to_gameplay(&rom_data);
    nes.set_button(0, BTN_RIGHT, true);

    let total_frames = 20000u32;
    let mut frame_data: Vec<(u32, f64)> = Vec::with_capacity(total_frames as usize);

    let baseline_avg = {
        let mut sum = 0.0;
        for _ in 0..300 {
            let t0 = Instant::now();
            nes.run_frame();
            sum += t0.elapsed().as_micros() as f64;
        }
        sum / 300.0
    };

    for frame in 300..total_frames {
        // Simulate gameplay
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

        let t0 = Instant::now();
        nes.run_frame();
        let dt = t0.elapsed().as_micros() as f64;
        frame_data.push((frame, dt));
    }

    // Analyze: find slow regions
    let spike_threshold = baseline_avg * 3.0;

    eprintln!("\n  Baseline: {:.0}µs/frame", baseline_avg);

    // Group consecutive slow frames into "slow regions"
    let mut regions: Vec<(u32, u32, f64)> = Vec::new(); // start, end, max_time
    let mut region_start: Option<u32> = None;
    let mut region_max = 0.0f64;

    for &(frame, dt) in &frame_data {
        if dt > spike_threshold {
            if region_start.is_none() {
                region_start = Some(frame);
            }
            region_max = region_max.max(dt);
        } else if let Some(start) = region_start {
            if frame > start + 5 { // allow small gaps
                regions.push((start, frame, region_max));
                region_start = None;
                region_max = 0.0;
            }
        }
    }
    if let Some(start) = region_start {
        regions.push((start, total_frames, region_max));
    }

    eprintln!("  Slow regions (>{:.0}µs threshold):", spike_threshold);
    for &(start, end, max) in &regions {
        let game_sec = start as f64 / 60.0;
        let duration = end - start;
        eprintln!("    Frames {:5}-{:5} ({:3} frames, ~{:.0}s into game): max {:.0}µs ({:.1}x)",
            start, end, duration, game_sec, max, max / baseline_avg);
    }

    if regions.is_empty() {
        eprintln!("    (none found)");
    }

    // Deep dive into the worst region
    if let Some(&(start, end, _)) = regions.iter().max_by(|a, b| a.2.partial_cmp(&b.2).unwrap()) {
        eprintln!("\n  Worst region detail (frames {}-{}):", start, end);

        // Rerun from scratch to this point and profile CPU cycles
        let mut nes2 = boot_to_gameplay(&rom_data);
        nes2.set_button(0, BTN_RIGHT, true);

        // Fast-forward to just before the slow region
        let target = start.saturating_sub(60);
        for f in 0..target {
            let action = f % 90;
            if action == 0 { nes2.set_button(0, BTN_A, true); }
            else if action == 10 { nes2.set_button(0, BTN_A, false); }
            else if action == 20 { nes2.set_button(0, BTN_B, true); }
            else if action == 28 { nes2.set_button(0, BTN_B, false); }
            if f % 600 == 300 {
                nes2.set_button(0, BTN_RIGHT, false);
                nes2.set_button(0, BTN_LEFT, true);
            } else if f % 600 == 0 {
                nes2.set_button(0, BTN_LEFT, false);
                nes2.set_button(0, BTN_RIGHT, true);
            }
            nes2.run_frame();
        }

        // Now profile individual frames around the slow region
        let profile_start = target;
        let profile_end = (end + 60).min(total_frames);

        for f in profile_start..profile_end {
            let action = f % 90;
            if action == 0 { nes2.set_button(0, BTN_A, true); }
            else if action == 10 { nes2.set_button(0, BTN_A, false); }
            else if action == 20 { nes2.set_button(0, BTN_B, true); }
            else if action == 28 { nes2.set_button(0, BTN_B, false); }

            // Count CPU steps in this frame
            let ppu_frame_before = nes2.bus.ppu.frame_count;
            let cpu_cycles_before = nes2.cpu.cycles;

            let t0 = Instant::now();
            nes2.run_frame();
            let dt = t0.elapsed().as_micros() as f64;

            let cpu_cycles = nes2.cpu.cycles - cpu_cycles_before;

            // Count sprites on screen (OAM analysis)
            let mut visible_sprites = 0u32;
            let scanline = nes2.bus.ppu.scanline as u16;
            let sprite_h: u16 = if nes2.bus.ppu.ctrl & 0x20 != 0 { 16 } else { 8 };
            for s in 0..64 {
                let y = nes2.bus.ppu.oam[s * 4] as u16;
                if y < 240 {
                    visible_sprites += 1;
                }
            }

            if dt > spike_threshold || f == profile_start || f == profile_end - 1 {
                let marker = if dt > spike_threshold { " <-- SLOW" } else { "" };
                eprintln!("    Frame {:5}: {:6.0}µs  cpu_cycles={:6}  sprites={:2}{}",
                    f, dt, cpu_cycles, visible_sprites, marker);
            }
        }
    }

    // ASSERTION: no single frame should take more than 10x baseline
    let worst = frame_data.iter().map(|&(_, dt)| dt).fold(0.0f64, f64::max);
    assert!(
        worst < baseline_avg * 10.0,
        "Worst frame ({:.0}µs) is {:.1}x baseline ({:.0}µs) — unacceptable spike",
        worst, worst / baseline_avg, baseline_avg
    );
}
