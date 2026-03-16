// Pixel-perfect test — verifies frame output matches reference hashes.
//
// Reference hashes were captured from the native Rust renderer with a
// deterministic input sequence (konami code -> gameplay with RIGHT held).
// Game data is compiled into the binary — no ROM file needed.

#[path = "../src/apu.rs"]
mod apu;
#[path = "../src/cartridge.rs"]
mod cartridge;
#[path = "../src/renderer.rs"]
mod renderer;
#[path = "../src/game.rs"]
mod game;

use game::Game;

const BTN_A: u8 = 0;
const BTN_B: u8 = 1;
const BTN_START: u8 = 3;
const BTN_UP: u8 = 4;
const BTN_DOWN: u8 = 5;
const BTN_LEFT: u8 = 6;
const BTN_RIGHT: u8 = 7;

// Reference hashes from the native Rust renderer (level 5 architecture).
// These verify deterministic output from the trace-replay game engine.
const REFERENCE_HASHES: &[(u32, u64)] = &[
    (  60, 0x06f6794a01b95e1d),
    ( 180, 0xdc946923f0f9d231),
    ( 500, 0x0e0dee516c09ffbb),
    ( 600, 0x789e33e8f0c321e5),
    ( 700, 0xa88c07b9476e2615),
    ( 800, 0x96d63225ea926325),
    ( 900, 0xd5688c972aedfca7),
    (1000, 0xe4852b3f5be7c30b),
    (1200, 0x7d43121f6c5e1bd5),
    (1500, 0x291237014fc3d64f),
    (2000, 0x319ea20845e86019),
    (2500, 0x3fc717cf701c11c3),
    (3000, 0x6bcb85bcde6c40f3),
];

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
    // Game data is compiled in — no ROM file needed
    let mut game = Game::new(&[], 0);
    let mut frame = 0u32;

    let tap = |game: &mut Game, frame: &mut u32, btn: u8, hold: u32, gap: u32| {
        game.set_button(0, btn, true);
        for _ in 0..hold { game.update(); *frame += 1; }
        game.set_button(0, btn, false);
        for _ in 0..gap { game.update(); *frame += 1; }
    };

    let mut check_idx = 0;
    let mut check = |frame: u32, game: &Game, idx: &mut usize| {
        while *idx < REFERENCE_HASHES.len() && REFERENCE_HASHES[*idx].0 == frame {
            let (_, expected) = REFERENCE_HASHES[*idx];
            let actual = framebuffer_hash(game.framebuffer());
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
        game.update();
        frame += 1;
        check(frame, &game, &mut check_idx);
    }

    // Konami code
    tap(&mut game, &mut frame, BTN_UP, 4, 6);
    check(frame, &game, &mut check_idx);
    tap(&mut game, &mut frame, BTN_UP, 4, 6);
    check(frame, &game, &mut check_idx);
    tap(&mut game, &mut frame, BTN_DOWN, 4, 6);
    check(frame, &game, &mut check_idx);
    tap(&mut game, &mut frame, BTN_DOWN, 4, 6);
    check(frame, &game, &mut check_idx);
    tap(&mut game, &mut frame, BTN_LEFT, 4, 6);
    check(frame, &game, &mut check_idx);
    tap(&mut game, &mut frame, BTN_RIGHT, 4, 6);
    check(frame, &game, &mut check_idx);
    tap(&mut game, &mut frame, BTN_LEFT, 4, 6);
    check(frame, &game, &mut check_idx);
    tap(&mut game, &mut frame, BTN_RIGHT, 4, 6);
    check(frame, &game, &mut check_idx);
    tap(&mut game, &mut frame, BTN_B, 4, 6);
    check(frame, &game, &mut check_idx);
    tap(&mut game, &mut frame, BTN_A, 4, 6);
    check(frame, &game, &mut check_idx);
    tap(&mut game, &mut frame, BTN_START, 4, 6);
    check(frame, &game, &mut check_idx);

    // Gameplay with RIGHT held
    game.set_button(0, BTN_RIGHT, true);

    while frame < 3001 {
        game.update();
        frame += 1;
        check(frame, &game, &mut check_idx);
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
    let mut hashes_a = Vec::new();
    let mut hashes_b = Vec::new();

    for hashes in [&mut hashes_a, &mut hashes_b] {
        let mut game = Game::new(&[], 0);
        for _ in 0..600 {
            game.update();
        }
        // Hash every 60th frame
        for _ in 0..10 {
            for _ in 0..60 {
                game.update();
            }
            hashes.push(framebuffer_hash(game.framebuffer()));
        }
    }

    assert_eq!(hashes_a, hashes_b, "Game is not deterministic across runs");
    eprintln!("  10 checkpoints matched across 2 independent runs");
}
