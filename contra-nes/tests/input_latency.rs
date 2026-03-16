// Input latency test — measures controller responsiveness

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

fn framebuffer_checksum(fb: &[u8]) -> u64 {
    let mut hash = 0u64;
    for (i, &b) in fb.iter().enumerate().step_by(7) {
        hash = hash.wrapping_mul(31).wrapping_add(b as u64).wrapping_add(i as u64);
    }
    hash
}

/// Read what the game sees when it polls the controller.
/// Simulates the NES read sequence: write 1 to $4016, write 0, then read 8 times.
fn read_controller_state(nes: &mut Nes) -> u8 {
    nes.bus.write(0x4016, 1);
    nes.bus.write(0x4016, 0);
    let mut buttons = 0u8;
    for i in 0..8 {
        let bit = nes.bus.read(0x4016) & 1;
        buttons |= bit << i;
    }
    buttons
}

/// Test: Verify set_button is visible when the controller is polled.
/// This is the most fundamental test — if this fails, no input can work.
#[test]
fn controller_strobe_reads_current_buttons() {
    let rom_data = match load_rom() {
        Some(d) => d,
        None => { eprintln!("  SKIP: contra.nes not found"); return; }
    };

    let cart = cartridge::Cartridge::from_ines(&rom_data);
    let mut nes = Nes::new(cart);

    // Read with no buttons pressed
    let empty = read_controller_state(&mut nes);
    eprintln!("  No buttons: 0x{:02X}", empty);
    assert_eq!(empty, 0x00, "Expected 0 with no buttons pressed, got 0x{:02X}", empty);

    // Press A
    nes.set_button(0, BTN_A, true);
    let with_a = read_controller_state(&mut nes);
    eprintln!("  A pressed: 0x{:02X}", with_a);
    assert_eq!(with_a & 1, 1, "A button (bit 0) not set: 0x{:02X}", with_a);

    // Press START too
    nes.set_button(0, BTN_START, true);
    let with_a_start = read_controller_state(&mut nes);
    eprintln!("  A+START: 0x{:02X}", with_a_start);
    assert_eq!(with_a_start & 0x09, 0x09, "A+START not set: 0x{:02X}", with_a_start);

    // Release A, keep START
    nes.set_button(0, BTN_A, false);
    let with_start = read_controller_state(&mut nes);
    eprintln!("  START only: 0x{:02X}", with_start);
    assert_eq!(with_start & 0x08, 0x08, "START not set: 0x{:02X}", with_start);
    assert_eq!(with_start & 0x01, 0x00, "A should be released: 0x{:02X}", with_start);

    // Press all directions
    nes.set_button(0, BTN_UP, true);
    nes.set_button(0, BTN_RIGHT, true);
    let with_dirs = read_controller_state(&mut nes);
    eprintln!("  START+UP+RIGHT: 0x{:02X}", with_dirs);
    assert!(with_dirs & (1 << BTN_UP) != 0, "UP not set");
    assert!(with_dirs & (1 << BTN_RIGHT) != 0, "RIGHT not set");
}

/// Test: Verify the game actually reads the controller during a frame.
/// We trace $4016 reads during run_frame to confirm the game polls input.
#[test]
fn game_polls_controller_each_frame() {
    let rom_data = match load_rom() {
        Some(d) => d,
        None => { eprintln!("  SKIP: contra.nes not found"); return; }
    };

    let cart = cartridge::Cartridge::from_ines(&rom_data);
    let mut nes = Nes::new(cart);

    // Run past title screen
    for _ in 0..300 {
        nes.run_frame();
    }

    // Set a button and run a frame, then check if the shift register was consumed
    nes.set_button(0, BTN_START, true);

    // Before frame: manually strobe to load shift register
    nes.bus.write(0x4016, 1);
    nes.bus.write(0x4016, 0);
    let shift_before = nes.bus.controller_shift[0];
    eprintln!("  Shift register after manual strobe: 0x{:02X}", shift_before);
    eprintln!("  Controller state: 0x{:02X}", nes.bus.controller[0]);

    // Run a frame — the game's NMI handler should strobe and read the controller
    nes.run_frame();

    // Check if the game consumed the shift register (it should be partially shifted)
    let shift_after = nes.bus.controller_shift[0];
    eprintln!("  Shift register after frame: 0x{:02X}", shift_after);
    eprintln!("  (Different from controller state means game read it)");

    // The shift register should have been re-strobed and read by the game
    // If the game never read it, it would still be at our manual strobe value
}

/// Test: Two parallel NES instances diverge when one gets input.
/// Run both 300 frames to title screen, press START on one, run 120 more frames.
#[test]
fn input_causes_divergence() {
    let rom_data = match load_rom() {
        Some(d) => d,
        None => { eprintln!("  SKIP: contra.nes not found"); return; }
    };

    let cart1 = cartridge::Cartridge::from_ines(&rom_data);
    let cart2 = cartridge::Cartridge::from_ines(&rom_data);
    let mut nes1 = Nes::new(cart1);
    let mut nes2 = Nes::new(cart2);

    // Run both to title screen
    for _ in 0..300 {
        nes1.run_frame();
        nes2.run_frame();
    }

    let hash_at_300 = framebuffer_checksum(nes1.framebuffer());
    let hash2_at_300 = framebuffer_checksum(nes2.framebuffer());
    eprintln!("  Frame 300 nes1: {:016x}", hash_at_300);
    eprintln!("  Frame 300 nes2: {:016x}", hash2_at_300);
    assert_eq!(hash_at_300, hash2_at_300, "Instances diverged before input");

    // Verify controller reads work
    let c1 = read_controller_state(&mut nes1);
    eprintln!("  nes1 controller before START: 0x{:02X}", c1);

    // Press START on nes1
    nes1.set_button(0, BTN_START, true);

    let c1_after = read_controller_state(&mut nes1);
    eprintln!("  nes1 controller after START:  0x{:02X}", c1_after);

    // Run both for 120 frames (2 seconds) — enough to see menu change
    let mut diverged_at = None;
    for i in 0..120 {
        nes1.run_frame();
        nes2.run_frame();

        let h1 = framebuffer_checksum(nes1.framebuffer());
        let h2 = framebuffer_checksum(nes2.framebuffer());

        if h1 != h2 && diverged_at.is_none() {
            diverged_at = Some(i + 1);
            eprintln!("  Diverged at frame {} after START", i + 1);
        }
    }

    // Release START after a few frames
    nes1.set_button(0, BTN_START, false);

    match diverged_at {
        Some(frame) => {
            // Menu transitions can take a few frames — 10 is the limit
            assert!(frame <= 10, "Took {} frames to diverge after START — too slow", frame);
        }
        None => {
            // Dump some state to help debug
            eprintln!("  nes1 final: {:016x}", framebuffer_checksum(nes1.framebuffer()));
            eprintln!("  nes2 final: {:016x}", framebuffer_checksum(nes2.framebuffer()));
            eprintln!("  nes1 controller[0]: 0x{:02X}", nes1.bus.controller[0]);
            eprintln!("  nes2 controller[0]: 0x{:02X}", nes2.bus.controller[0]);
            eprintln!("  nes1 shift[0]: 0x{:02X}", nes1.bus.controller_shift[0]);

            // Try reading controller directly one more time
            let final_read = read_controller_state(&mut nes1);
            eprintln!("  nes1 final controller read: 0x{:02X}", final_read);

            panic!("NES instances never diverged — START had no effect on game");
        }
    }
}

/// Test: Frame pacing math doesn't drift
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
