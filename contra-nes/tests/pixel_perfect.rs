// Pixel-perfect test — verifies frame output matches reference hashes.
//
// Reference hashes were captured from the emulator with a deterministic
// input sequence (konami code → gameplay with RIGHT held).
// After migration to native Rust, these must still match exactly.

// During migration, the game module structure will change.
// This test imports whatever the current game entry point is.
// Phase 1 (emulator): imports cpu/ppu/bus/nes
// Phase 2+ (native): imports game module directly

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

const BTN_A: u8 = 0;
const BTN_B: u8 = 1;
const BTN_START: u8 = 3;
const BTN_UP: u8 = 4;
const BTN_DOWN: u8 = 5;
const BTN_LEFT: u8 = 6;
const BTN_RIGHT: u8 = 7;

// Reference frame hashes (generated from emulator)
// Input sequence: 250 frames boot, konami code, START, then hold RIGHT
const REFERENCE_HASHES: &[(u32, u64)] = &[
    (  60, 0x9a3b11d821d935c7),
    ( 180, 0x109942579bb23597),
    ( 500, 0x6a061d431cbc87b5),
    ( 600, 0xe187d3da08407705),
    ( 700, 0x72cc3b6666866903),
    ( 800, 0xe187d3da08407705),
    ( 900, 0x366cbb75eabc753b),
    (1000, 0x27c842dd27839dbd),
    (1200, 0x7cecfd765df5d9df),
    (1500, 0xae4ab7eff4f93647),
    (2000, 0x33d4b4dd3fc800a7),
    (2500, 0x2c7c67993f2052fd),
    (3000, 0xee9bb671a6a45505),
];

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

fn framebuffer_hash(fb: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in fb.iter() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Replay the deterministic input sequence and verify every checkpoint
/// frame matches the reference hash exactly.
#[test]
fn all_frames_match_reference() {
    let rom_data = match load_rom() {
        Some(d) => d,
        None => { eprintln!("  SKIP: contra.nes not found"); return; }
    };

    let cart = cartridge::Cartridge::from_ines(&rom_data);
    let mut nes = Nes::new(cart);
    let mut frame = 0u32;

    let tap = |nes: &mut Nes, frame: &mut u32, btn: u8, hold: u32, gap: u32| {
        nes.set_button(0, btn, true);
        for _ in 0..hold { nes.run_frame(); *frame += 1; }
        nes.set_button(0, btn, false);
        for _ in 0..gap { nes.run_frame(); *frame += 1; }
    };

    let mut check_idx = 0;
    let mut check = |frame: u32, nes: &Nes, idx: &mut usize| {
        while *idx < REFERENCE_HASHES.len() && REFERENCE_HASHES[*idx].0 == frame {
            let (_, expected) = REFERENCE_HASHES[*idx];
            let actual = framebuffer_hash(nes.framebuffer());
            assert_eq!(
                actual, expected,
                "Frame {} hash mismatch: got 0x{:016x}, expected 0x{:016x}",
                frame, actual, expected
            );
            *idx += 1;
        }
    };

    // Boot
    for _ in 0..250 {
        nes.run_frame();
        frame += 1;
        check(frame, &nes, &mut check_idx);
    }

    // Konami code
    tap(&mut nes, &mut frame, BTN_UP, 4, 6);
    check(frame, &nes, &mut check_idx);
    tap(&mut nes, &mut frame, BTN_UP, 4, 6);
    check(frame, &nes, &mut check_idx);
    tap(&mut nes, &mut frame, BTN_DOWN, 4, 6);
    check(frame, &nes, &mut check_idx);
    tap(&mut nes, &mut frame, BTN_DOWN, 4, 6);
    check(frame, &nes, &mut check_idx);
    tap(&mut nes, &mut frame, BTN_LEFT, 4, 6);
    check(frame, &nes, &mut check_idx);
    tap(&mut nes, &mut frame, BTN_RIGHT, 4, 6);
    check(frame, &nes, &mut check_idx);
    tap(&mut nes, &mut frame, BTN_LEFT, 4, 6);
    check(frame, &nes, &mut check_idx);
    tap(&mut nes, &mut frame, BTN_RIGHT, 4, 6);
    check(frame, &nes, &mut check_idx);
    tap(&mut nes, &mut frame, BTN_B, 4, 6);
    check(frame, &nes, &mut check_idx);
    tap(&mut nes, &mut frame, BTN_A, 4, 6);
    check(frame, &nes, &mut check_idx);
    tap(&mut nes, &mut frame, BTN_START, 4, 6);
    check(frame, &nes, &mut check_idx);

    // Gameplay with RIGHT held
    nes.set_button(0, BTN_RIGHT, true);

    while frame < 3001 {
        nes.run_frame();
        frame += 1;
        check(frame, &nes, &mut check_idx);
    }

    assert_eq!(
        check_idx,
        REFERENCE_HASHES.len(),
        "Only checked {}/{} reference frames",
        check_idx,
        REFERENCE_HASHES.len()
    );
    eprintln!("  All {} reference frames matched", check_idx);
}

/// Determinism: two runs with identical inputs produce identical output.
#[test]
fn deterministic_across_runs() {
    let rom_data = match load_rom() {
        Some(d) => d,
        None => { eprintln!("  SKIP: contra.nes not found"); return; }
    };

    let mut hashes_a = Vec::new();
    let mut hashes_b = Vec::new();

    for hashes in [&mut hashes_a, &mut hashes_b] {
        let cart = cartridge::Cartridge::from_ines(&rom_data);
        let mut nes = Nes::new(cart);
        for _ in 0..600 {
            nes.run_frame();
        }
        // Hash every 60th frame
        for _ in 0..10 {
            for _ in 0..60 {
                nes.run_frame();
            }
            hashes.push(framebuffer_hash(nes.framebuffer()));
        }
    }

    assert_eq!(hashes_a, hashes_b, "Emulator is not deterministic across runs");
    eprintln!("  10 checkpoints matched across 2 independent runs");
}
