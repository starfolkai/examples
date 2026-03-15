# contra-nes

Native NES emulator in Rust. Plays Contra. Zero external dependencies in the hot path.

Built for speed: **500+ fps** (8x NES realtime) with full CPU + PPU emulation.

## Build

```bash
cargo build --release
```

## Play

```bash
# Interactive mode — plays in your terminal
./target/release/contra-nes --rom contra.nes --play --scale 4

# Controls:
#   WASD  = D-pad (move/aim)
#   J     = A button (jump)
#   K     = B button (shoot)
#   Enter = Start
#   Space = Select
#   Q     = Quit
```

The game auto-enters the Konami code (30 lives) and starts for you. After the intro, keyboard control hands off to you.

## Other modes

```bash
# Watch the AI autoplay in terminal
./target/release/contra-nes --rom contra.nes --terminal --scale 4

# Benchmark
./target/release/contra-nes --rom contra.nes --benchmark --frames 5000

# Export frames as BMP/PPM
./target/release/contra-nes --rom contra.nes --export --interval 100

# Pipe raw RGB24 to ffmpeg for video
./target/release/contra-nes --rom contra.nes --pipe --frames 3600 | \
  ffmpeg -f rawvideo -pix_fmt rgb24 -s 256x240 -r 60 -i - \
  -c:v libx264 -pix_fmt yuv420p contra.mp4
```

## Architecture

~1400 lines of Rust across 6 modules:

| Module | What |
|--------|------|
| `cpu.rs` | MOS 6502 — all legal opcodes + common illegals (LAX, SAX, DCP, ISB, SLO, RLA, SRE, RRA) |
| `ppu.rs` | 2C02 PPU — scanline-accurate BG + sprites, attribute tables, scroll, sprite 0 hit, NMI |
| `cartridge.rs` | iNES loader + Mapper 2 (UxROM) bank switching |
| `bus.rs` | Memory map, OAM DMA, controller I/O |
| `nes.rs` | System clock: 3 PPU ticks per CPU cycle |
| `main.rs` | CLI, terminal renderer (truecolor ANSI half-blocks), input handling |

Only dependency is `libc` for raw terminal mode. The emulation core is zero-dependency.

## ROM

You need a `contra.nes` ROM file (Contra USA, iNES format, Mapper 2). Not included.
