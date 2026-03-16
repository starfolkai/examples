// Native architecture tests — verify the codebase uses native Rust
// game constructs instead of NES hardware emulation.
//
// These tests check the SOURCE CODE structure, not runtime behavior.
// They define the target architecture for the migration:
//   - No CPU interpreter (game logic is Rust)
//   - No PPU shift registers (rendering is direct tile/sprite drawing)
//   - Game state lives in typed Rust structs
//   - Game data (levels, tiles, sprites) are Rust const arrays
//
// Initially these will FAIL. Migration is complete when they all pass.

use std::path::Path;
use std::fs;

const SRC_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src");

fn src_file_exists(name: &str) -> bool {
    Path::new(SRC_DIR).join(name).exists()
}

fn read_src(name: &str) -> Option<String> {
    std::fs::read_to_string(Path::new(SRC_DIR).join(name)).ok()
}

// ── Phase 1: No CPU interpreter ──

/// The game logic must be native Rust, not 6502 interpretation.
/// cpu.rs should not exist (or should not contain opcode dispatch).
#[test]
fn no_6502_cpu_interpreter() {
    if let Some(contents) = read_src("cpu.rs") {
        // cpu.rs exists — check it doesn't have opcode dispatch
        assert!(
            !contents.contains("0x69") || !contents.contains("ADC"),
            "cpu.rs still contains 6502 opcode dispatch — game logic should be native Rust"
        );
        assert!(
            !contents.contains("fn step("),
            "cpu.rs still has step() — remove the CPU interpreter"
        );
    }
    // If cpu.rs doesn't exist, test passes
}

// ── Phase 2: Native game modules exist ──

/// The game must have a top-level game module with typed state.
#[test]
fn has_game_module() {
    assert!(
        src_file_exists("game.rs"),
        "src/game.rs missing — need a Game struct with update()/render()"
    );
    let contents = read_src("game.rs").unwrap();
    assert!(
        contents.contains("struct Game"),
        "game.rs must define `struct Game`"
    );
    assert!(
        contents.contains("fn update("),
        "Game must have an update() method"
    );
    assert!(
        contents.contains("fn render("),
        "Game must have a render() method"
    );
}

/// Player state must be a typed Rust struct, not RAM addresses.
#[test]
fn has_player_module() {
    assert!(
        src_file_exists("player.rs"),
        "src/player.rs missing — need a Player struct with position, lives, weapon"
    );
    let contents = read_src("player.rs").unwrap();
    assert!(
        contents.contains("struct Player"),
        "player.rs must define `struct Player`"
    );
}

/// Level data must be Rust data structures, not nametable bytes.
#[test]
fn has_level_module() {
    assert!(
        src_file_exists("level.rs"),
        "src/level.rs missing — need level layout data and scrolling logic"
    );
    let contents = read_src("level.rs").unwrap();
    assert!(
        contents.contains("struct Level") || contents.contains("struct Stage"),
        "level.rs must define a Level or Stage struct"
    );
}

/// Enemy types must be Rust enums/structs, not OAM byte patterns.
#[test]
fn has_enemy_module() {
    assert!(
        src_file_exists("enemies.rs"),
        "src/enemies.rs missing — need enemy types and AI"
    );
    let contents = read_src("enemies.rs").unwrap();
    assert!(
        contents.contains("struct Enemy") || contents.contains("enum EnemyType"),
        "enemies.rs must define Enemy struct or EnemyType enum"
    );
}

// ── Phase 3: Game data is extracted ──

/// Tile patterns must be const arrays, not loaded from CHR RAM at runtime.
#[test]
fn has_tile_data() {
    assert!(
        src_file_exists("data/tiles.rs") || src_file_exists("data.rs"),
        "src/data/tiles.rs or src/data.rs missing — need extracted tile patterns"
    );
}

/// Level layouts must be const data, not decompressed from ROM at runtime.
#[test]
fn has_level_data() {
    assert!(
        src_file_exists("data/levels.rs") || src_file_exists("data.rs"),
        "src/data/levels.rs or src/data.rs missing — need extracted level layouts"
    );
}

// ── Phase 4: No NES hardware emulation ──

/// The PPU must not contain shift register rendering.
#[test]
fn no_ppu_shift_registers() {
    if let Some(contents) = read_src("ppu.rs") {
        assert!(
            !contents.contains("bg_lo_shift"),
            "ppu.rs still has shift registers — use direct tile rendering"
        );
        assert!(
            !contents.contains("fn tick("),
            "ppu.rs still has per-dot tick() — use scanline or frame rendering"
        );
    }
    // If ppu.rs doesn't exist, that's fine too (fully native)
}

/// The bus must not contain NES memory map emulation.
#[test]
fn no_nes_memory_map() {
    if let Some(contents) = read_src("bus.rs") {
        assert!(
            !contents.contains("0x2000..=0x3FFF"),
            "bus.rs still maps PPU registers — game should access state directly"
        );
        assert!(
            !contents.contains("0x8000..=0xFFFF"),
            "bus.rs still maps cartridge ROM — game data should be Rust consts"
        );
    }
    // If bus.rs doesn't exist, test passes
}

/// No NES ROM file dependency at runtime.
#[test]
fn no_rom_file_dependency() {
    let main_rs = read_src("main.rs").expect("main.rs must exist");
    assert!(
        !main_rs.contains(".nes\"") && !main_rs.contains("from_ines"),
        "main.rs still loads a .nes ROM file — game data should be compiled in"
    );
}

// ── Phase 5: Renderer is native ──

/// Rendering must use direct tile/sprite drawing, not PPU emulation.
#[test]
fn has_native_renderer() {
    assert!(
        src_file_exists("renderer.rs"),
        "src/renderer.rs missing — need a native tile/sprite renderer"
    );
    let contents = read_src("renderer.rs").unwrap();
    assert!(
        contents.contains("fn render"),
        "renderer.rs must have a render function"
    );
}

// ── Level 5: Fully native — no NES emulation anywhere ──

/// Scan ALL source files for NES emulation patterns.
/// Level 5 means zero NES hardware emulation in the entire codebase.
#[test]
fn level5_no_emulation_in_any_source_file() {
    let src = Path::new(SRC_DIR);
    let mut failures = Vec::new();

    for entry in fs::read_dir(src).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().map_or(true, |e| e != "rs") { continue; }
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        let contents = fs::read_to_string(&path).unwrap();

        // No 6502 opcode dispatch (match arms like `0xA9 =>`)
        if contents.contains("0xA9 =>") || contents.contains("0x69 =>")
            || contents.contains("0x4C =>") || contents.contains("0x20 =>")
        {
            failures.push(format!("{}: contains 6502 opcode dispatch", name));
        }

        // No CPU register struct (a, x, y, sp, pc together)
        if contents.contains("pc: u16")
            && contents.contains("sp: u8")
            && contents.contains("a: u8")
        {
            failures.push(format!("{}: contains 6502 CPU register struct", name));
        }

        // No NES memory map ranges
        if contents.contains("0x2000..=0x3FFF") || contents.contains("0x8000..=0xFFFF") {
            failures.push(format!("{}: contains NES memory map", name));
        }

        // No PPU shift registers
        if contents.contains("bg_lo_shift") || contents.contains("bg_hi_shift") {
            failures.push(format!("{}: contains PPU shift registers", name));
        }

        // No per-dot PPU tick
        if contents.contains("fn tick(") {
            failures.push(format!("{}: contains per-dot tick()", name));
        }

        // No OAM byte array (NES sprite hardware)
        if contents.contains("oam: [u8; 256]") {
            failures.push(format!("{}: contains raw OAM array", name));
        }

        // No nametable RAM (NES VRAM)
        if contents.contains("nt_ram: [u8; 2048]") {
            failures.push(format!("{}: contains nametable RAM array", name));
        }

        // No NES palette RAM
        if contents.contains("palette: [u8; 32]") {
            failures.push(format!("{}: contains NES palette RAM", name));
        }

        // Giant match blocks (>80 arms = likely opcode dispatch)
        let match_arms = contents.matches("=> {").count()
            + contents.matches("=> { let").count();
        if match_arms > 80 {
            failures.push(format!(
                "{}: has {} match arms (likely opcode dispatch)", name, match_arms
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "Level 5 violations found:\n  {}",
        failures.join("\n  ")
    );
}

/// Game logic must not be driven by a CPU interpreter.
/// game.rs should use native Rust control flow, not instruction stepping.
#[test]
fn level5_game_uses_native_logic() {
    let contents = read_src("game.rs").unwrap();
    assert!(
        !contents.contains("fn step("),
        "game.rs still has CPU step() — game logic should be native Rust"
    );
    assert!(
        !contents.contains("fn nes_step("),
        "game.rs still has nes_step() — remove the NES emulation loop"
    );
    assert!(
        !contents.contains("nmi_pending"),
        "game.rs still uses NMI — native game doesn't need interrupts"
    );
}

/// Renderer must do real work, not delegate to PPU emulation.
#[test]
fn level5_renderer_draws_tiles() {
    let contents = read_src("renderer.rs").unwrap();
    assert!(
        contents.contains("fn draw_tile") || contents.contains("fn render_scanline")
            || contents.contains("fn render_background"),
        "renderer.rs must have tile drawing functions, not just delegate to PPU"
    );
}
