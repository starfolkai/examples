# contra-nes

Native NES emulator in Rust. Plays Contra in a window. 500+ fps.

## Build

```bash
cargo build --release
```

On Linux you need X11 and ALSA dev headers: `sudo apt install libx11-dev libxkbcommon-dev libasound2-dev`

## Play

```bash
# Opens a window — this is the default mode
./target/release/contra-nes --rom contra.nes

# Scale up (default 4x)
./target/release/contra-nes --rom contra.nes --scale 3
```

The game auto-enters the Konami code (30 lives) and starts. After the intro, you have full control.

### Controls

| Key | NES Button |
|-----|------------|
| Arrow keys or WASD | D-pad |
| Z or J | A (jump) |
| X or K | B (shoot) |
| Enter | Start |
| Space | Select |
| Escape | Quit |

## Other modes

```bash
# Benchmark (headless, max speed)
./target/release/contra-nes --rom contra.nes --benchmark --frames 5000

# Terminal rendering (ANSI truecolor, no window needed)
./target/release/contra-nes --rom contra.nes --terminal --scale 4

# Export frames as BMP/PPM
./target/release/contra-nes --rom contra.nes --export --interval 100

# Pipe raw RGB24 to ffmpeg for video
./target/release/contra-nes --rom contra.nes --pipe --frames 3600 | \
  ffmpeg -f rawvideo -pix_fmt rgb24 -s 256x240 -r 60 -i - \
  -c:v libx264 -pix_fmt yuv420p contra.mp4
```

## Architecture

~2000 lines of Rust across 7 modules:

| Module | What |
|--------|------|
| `cpu.rs` | MOS 6502 — all legal opcodes + common illegals (LAX, SAX, DCP, ISB, SLO, RLA, SRE, RRA) |
| `ppu.rs` | 2C02 PPU — scanline-accurate BG + sprites, attribute tables, scroll, sprite 0 hit, NMI |
| `apu.rs` | 2A03 APU — pulse 1 & 2, triangle, noise, DMC, frame counter, nonlinear mixer |
| `cartridge.rs` | iNES loader + Mapper 2 (UxROM) bank switching |
| `bus.rs` | Memory map, OAM DMA, APU registers, controller I/O |
| `nes.rs` | System clock: 3 PPU ticks per CPU cycle, APU clocking |
| `main.rs` | CLI, windowed display (minifb), audio output (cpal), input handling |

## ROM

You need a `contra.nes` ROM file (Contra USA, iNES format, Mapper 2). Not included.
