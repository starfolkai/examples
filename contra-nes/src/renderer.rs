/// Native tile/sprite renderer for NES-format graphics.
///
/// Draws directly to an RGB24 framebuffer without any NES PPU emulation —
/// no shift registers, no per-dot tick, no register emulation.

pub const SCREEN_WIDTH: usize = 256;
pub const SCREEN_HEIGHT: usize = 240;
pub const FB_SIZE: usize = SCREEN_WIDTH * SCREEN_HEIGHT * 3;

/// NES master palette: 64 entries, each an (R, G, B) triple.
/// Based on the commonly used "2C02" palette.
#[rustfmt::skip]
pub const NES_PALETTE: [(u8, u8, u8); 64] = [
    (0x62, 0x62, 0x62), (0x00, 0x2E, 0x98), (0x12, 0x12, 0xA7), (0x3B, 0x00, 0xA4),
    (0x5B, 0x00, 0x89), (0x6B, 0x00, 0x5A), (0x6A, 0x00, 0x1E), (0x55, 0x12, 0x00),
    (0x35, 0x28, 0x00), (0x10, 0x3C, 0x00), (0x00, 0x42, 0x00), (0x00, 0x3C, 0x14),
    (0x00, 0x32, 0x3E), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),

    (0xAB, 0xAB, 0xAB), (0x0D, 0x57, 0xFF), (0x4B, 0x30, 0xFF), (0x84, 0x18, 0xE0),
    (0xAE, 0x17, 0xBF), (0xC2, 0x18, 0x5B), (0xC0, 0x26, 0x00), (0x9E, 0x41, 0x00),
    (0x6D, 0x5C, 0x00), (0x38, 0x71, 0x00), (0x0D, 0x7B, 0x00), (0x00, 0x76, 0x2D),
    (0x00, 0x6C, 0x71), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),

    (0xFF, 0xFF, 0xFF), (0x53, 0xAE, 0xFF), (0x90, 0x85, 0xFF), (0xCB, 0x6D, 0xFF),
    (0xF6, 0x6A, 0xFF), (0xFF, 0x6A, 0xAE), (0xFF, 0x7C, 0x63), (0xE8, 0x95, 0x21),
    (0xBA, 0xAB, 0x00), (0x7C, 0xC7, 0x00), (0x4F, 0xD0, 0x10), (0x30, 0xCB, 0x72),
    (0x38, 0xC1, 0xCC), (0x3C, 0x3C, 0x3C), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),

    (0xFF, 0xFF, 0xFF), (0xB6, 0xDA, 0xFF), (0xCE, 0xCA, 0xFF), (0xE7, 0xC2, 0xFF),
    (0xFB, 0xC0, 0xFF), (0xFF, 0xC0, 0xDB), (0xFF, 0xC8, 0xB7), (0xF5, 0xD2, 0x9B),
    (0xE0, 0xDD, 0x8D), (0xC5, 0xE9, 0x8B), (0xB0, 0xED, 0x96), (0xA2, 0xEB, 0xB0),
    (0xA4, 0xE6, 0xDB), (0xA8, 0xA8, 0xA8), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),
];

/// Look up an NES palette index and return (R, G, B).
#[inline(always)]
fn nes_color(index: u8) -> (u8, u8, u8) {
    NES_PALETTE[(index & 0x3F) as usize]
}

/// Write an RGB triple into the framebuffer at (screen_x, screen_y).
#[inline(always)]
fn put_pixel(fb: &mut [u8], screen_x: usize, screen_y: usize, r: u8, g: u8, b: u8) {
    let offset = (screen_y * SCREEN_WIDTH + screen_x) * 3;
    if offset + 2 < fb.len() {
        fb[offset] = r;
        fb[offset + 1] = g;
        fb[offset + 2] = b;
    }
}

/// Read the 2-bit pixel value from a CHR tile at a given (fine_x, fine_y).
///
/// `chr` is the full CHR ROM, `tile_addr` is the byte offset of the tile's
/// first plane-0 byte (each tile is 16 bytes: 8 low-plane + 8 high-plane).
#[inline(always)]
fn decode_tile_pixel(chr: &[u8], tile_addr: usize, fine_x: usize, fine_y: usize) -> u8 {
    let lo = chr.get(tile_addr + fine_y).copied().unwrap_or(0);
    let hi = chr.get(tile_addr + fine_y + 8).copied().unwrap_or(0);
    let shift = 7 - fine_x;
    ((lo >> shift) & 1) | (((hi >> shift) & 1) << 1)
}

/// Draw a single 8×8 tile into the framebuffer at the given screen position.
///
/// Pixels with color index 0 (transparent) are skipped.
/// `chr` is the full CHR data; `tile_addr` is the byte offset of the tile.
/// `pal` is a 4-byte slice holding the palette entries for this tile.
pub fn draw_tile(
    fb: &mut [u8],
    chr: &[u8],
    tile_addr: usize,
    pal: &[u8; 4],
    dest_x: i16,
    dest_y: i16,
    flip_h: bool,
    flip_v: bool,
) {
    for row in 0u8..8 {
        let screen_y = dest_y as i32 + row as i32;
        if screen_y < 0 || screen_y >= SCREEN_HEIGHT as i32 {
            continue;
        }
        let fine_y = if flip_v { 7 - row as usize } else { row as usize };
        for col in 0u8..8 {
            let screen_x = dest_x as i32 + col as i32;
            if screen_x < 0 || screen_x >= SCREEN_WIDTH as i32 {
                continue;
            }
            let fine_x = if flip_h { 7 - col as usize } else { col as usize };
            let pixel = decode_tile_pixel(chr, tile_addr, fine_x, fine_y);
            if pixel == 0 {
                continue;
            }
            let nes_idx = pal[pixel as usize];
            let (r, g, b) = nes_color(nes_idx);
            put_pixel(fb, screen_x as usize, screen_y as usize, r, g, b);
        }
    }
}

/// Render one scanline of the background layer.
///
/// This processes a single row of the 256×240 output, reading from the
/// nametable with scrolling applied. Called by `render_background` for each
/// visible row.
///
/// * `nametable` — 2 KB: two 1024-byte nametables laid out for vertical
///   mirroring (horizontal scrolling).
/// * `palette` — 32 bytes of NES palette RAM (indices into the master palette).
/// * `chr` — full CHR ROM/RAM.
/// * `bg_pattern_base` — 0 or 0x1000, selected by PPUCTRL bit 4.
fn render_scanline(
    fb: &mut [u8],
    chr: &[u8],
    nametable: &[u8],
    palette: &[u8],
    scroll_x: u16,
    scroll_y: u16,
    bg_pattern_base: usize,
    screen_y: usize,
) {
    let effective_y = ((scroll_y as usize) + screen_y) % 480;
    let tile_row = effective_y / 8;
    let fine_y = effective_y % 8;

    // Vertical nametable select (wraps at row 30 for NES nametable layout).
    let nt_row_select = if tile_row >= 30 { 1 } else { 0 };
    let tile_row_in_nt = tile_row % 30;

    let row_offset = screen_y * SCREEN_WIDTH * 3;

    for screen_x in 0..SCREEN_WIDTH {
        let effective_x = ((scroll_x as usize) + screen_x) % 512;
        let tile_col = effective_x / 8;
        let fine_x = effective_x % 8;

        // Horizontal nametable select (vertical mirroring: left/right).
        let nt_select = tile_col / 32;
        let tile_col_in_nt = tile_col % 32;

        // For vertical mirroring, both horizontal nametables mirror, so the
        // real nametable index is the horizontal select XOR'd with the vertical.
        // In a simple two-nametable horizontal-scroll layout we just use
        // nt_select for horizontal and ignore vertical (both halves mirror
        // vertically). To support 2 KB with wrapping:
        let nt_base = (nt_select ^ nt_row_select) * 1024;

        // Tile index from the nametable.
        let tile_index = *nametable
            .get(nt_base + tile_row_in_nt * 32 + tile_col_in_nt)
            .unwrap_or(&0) as usize;

        // Decode pixel from CHR.
        let tile_addr = bg_pattern_base + tile_index * 16;
        let pixel = decode_tile_pixel(chr, tile_addr, fine_x, fine_y);

        // Attribute byte → palette group.
        let attr_offset =
            nt_base + 960 + (tile_row_in_nt / 4) * 8 + (tile_col_in_nt / 4);
        let attr_byte = nametable.get(attr_offset).copied().unwrap_or(0);
        let shift = ((tile_row_in_nt / 2) & 1) * 4 + ((tile_col_in_nt / 2) & 1) * 2;
        let palette_group = ((attr_byte >> shift) & 3) as usize;

        // Resolve color: pixel 0 is always backdrop (palette[0]).
        let nes_idx = if pixel == 0 {
            palette.get(0).copied().unwrap_or(0x0F)
        } else {
            palette
                .get(palette_group * 4 + pixel as usize)
                .copied()
                .unwrap_or(0x0F)
        };

        let (r, g, b) = nes_color(nes_idx);
        let offset = row_offset + screen_x * 3;
        if offset + 2 < fb.len() {
            fb[offset] = r;
            fb[offset + 1] = g;
            fb[offset + 2] = b;
        }
    }
}

/// Render the full background layer into the framebuffer.
///
/// * `fb` — mutable RGB24 framebuffer, must be at least `FB_SIZE` bytes.
/// * `chr` — CHR ROM/RAM containing pattern tables.
/// * `nametable` — 2 KB of nametable data (two 1024-byte tables).
/// * `palette` — 32 bytes of palette RAM.
/// * `scroll_x`, `scroll_y` — pixel scroll offsets.
/// * `ctrl` — PPUCTRL register value (bit 4 selects BG pattern table).
/// * `mask` — PPUMASK register value (bit 3 enables BG rendering).
pub fn render_background(
    fb: &mut [u8],
    chr: &[u8],
    nametable: &[u8],
    palette: &[u8],
    scroll_x: u16,
    scroll_y: u16,
    ctrl: u8,
    mask: u8,
) {
    // If background rendering is disabled (PPUMASK bit 3), fill with backdrop.
    if mask & 0x08 == 0 {
        let backdrop_idx = palette.first().copied().unwrap_or(0x0F);
        let (r, g, b) = nes_color(backdrop_idx);
        for y in 0..SCREEN_HEIGHT {
            for x in 0..SCREEN_WIDTH {
                put_pixel(fb, x, y, r, g, b);
            }
        }
        return;
    }

    let bg_pattern_base: usize = if ctrl & 0x10 != 0 { 0x1000 } else { 0 };

    for y in 0..SCREEN_HEIGHT {
        render_scanline(
            fb,
            chr,
            nametable,
            palette,
            scroll_x,
            scroll_y,
            bg_pattern_base,
            y,
        );
    }
}

/// Render all sprites on top of (or behind) the existing background in the
/// framebuffer.
///
/// * `sprites` — slice of (y, tile_index, attributes, x) tuples in OAM order.
/// * `ctrl` — PPUCTRL register (bit 3 = sprite pattern table for 8×8 mode,
///   bit 5 = 8×16 sprite mode).
/// * `mask` — PPUMASK register (bit 4 enables sprite rendering).
///
/// Sprites are drawn in **reverse** order so that lower-indexed sprites have
/// higher priority (they overwrite later sprites).
pub fn render_sprites(
    fb: &mut [u8],
    chr: &[u8],
    sprites: &[(u8, u8, u8, u8)],
    palette: &[u8],
    ctrl: u8,
    mask: u8,
) {
    // If sprite rendering is disabled (PPUMASK bit 4), do nothing.
    if mask & 0x10 == 0 {
        return;
    }

    let is_8x16 = ctrl & 0x20 != 0;
    let sprite_pattern_base_8x8: usize = if ctrl & 0x08 != 0 { 0x1000 } else { 0 };

    // Draw in reverse order so that sprite 0 has highest priority.
    for &(raw_y, tile_index, attributes, x_pos) in sprites.iter().rev() {
        let sprite_y = raw_y as i16 + 1; // NES convention: display at y+1
        let sprite_x = x_pos as i16;
        let flip_h = attributes & 0x40 != 0;
        let flip_v = attributes & 0x80 != 0;
        let behind_bg = attributes & 0x20 != 0;
        let pal_group = (attributes & 3) as usize + 4; // sprite palettes at index 4..7

        // Build the 4-entry palette for this sprite.
        let pal_entries = [
            0, // index 0 is transparent (unused for sprites)
            palette.get(pal_group * 4 + 1).copied().unwrap_or(0x0F),
            palette.get(pal_group * 4 + 2).copied().unwrap_or(0x0F),
            palette.get(pal_group * 4 + 3).copied().unwrap_or(0x0F),
        ];

        let sprite_height: usize = if is_8x16 { 16 } else { 8 };

        // Determine pattern table address(es).
        let (top_tile_addr, bottom_tile_addr) = if is_8x16 {
            // 8×16: bit 0 of tile_index selects pattern table.
            let bank = (tile_index & 1) as usize * 0x1000;
            let base_tile = (tile_index & 0xFE) as usize;
            (bank + base_tile * 16, bank + (base_tile + 1) * 16)
        } else {
            let addr = sprite_pattern_base_8x8 + tile_index as usize * 16;
            (addr, addr) // bottom unused for 8×8
        };

        for row in 0..sprite_height {
            let screen_y = sprite_y as i32 + row as i32;
            if screen_y < 0 || screen_y >= SCREEN_HEIGHT as i32 {
                continue;
            }

            let effective_row = if flip_v {
                sprite_height - 1 - row
            } else {
                row
            };

            // For 8×16, top half uses top tile, bottom half uses bottom tile.
            let (tile_addr, fine_y) = if is_8x16 {
                if effective_row < 8 {
                    (top_tile_addr, effective_row)
                } else {
                    (bottom_tile_addr, effective_row - 8)
                }
            } else {
                (top_tile_addr, effective_row)
            };

            for col in 0..8usize {
                let screen_x = sprite_x as i32 + col as i32;
                if screen_x < 0 || screen_x >= SCREEN_WIDTH as i32 {
                    continue;
                }

                let fine_x = if flip_h { 7 - col } else { col };
                let pixel = decode_tile_pixel(chr, tile_addr, fine_x, fine_y);
                if pixel == 0 {
                    continue; // transparent
                }

                let sx = screen_x as usize;
                let sy = screen_y as usize;

                // Priority: if behind_bg and the existing background pixel
                // is not the backdrop color, skip drawing.
                if behind_bg {
                    let fb_offset = (sy * SCREEN_WIDTH + sx) * 3;
                    if fb_offset + 2 < fb.len() {
                        let backdrop_idx = palette.first().copied().unwrap_or(0x0F);
                        let (br, bg_g, bb) = nes_color(backdrop_idx);
                        let existing_r = fb[fb_offset];
                        let existing_g = fb[fb_offset + 1];
                        let existing_b = fb[fb_offset + 2];
                        if existing_r != br || existing_g != bg_g || existing_b != bb {
                            continue;
                        }
                    }
                }

                let nes_idx = pal_entries[pixel as usize];
                let (r, g, b) = nes_color(nes_idx);
                put_pixel(fb, sx, sy, r, g, b);
            }
        }
    }
}

/// Convenience function that renders a complete frame: background first, then
/// sprites on top.
///
/// All parameters are forwarded to `render_background` and `render_sprites`.
pub fn render_frame(
    fb: &mut [u8],
    chr: &[u8],
    nametable: &[u8],
    palette: &[u8],
    sprites: &[(u8, u8, u8, u8)],
    scroll_x: u16,
    scroll_y: u16,
    ctrl: u8,
    mask: u8,
) {
    render_background(fb, chr, nametable, palette, scroll_x, scroll_y, ctrl, mask);
    render_sprites(fb, chr, sprites, palette, ctrl, mask);
}

/// Legacy wrapper kept for compatibility with existing call sites.
pub struct Renderer;

impl Renderer {
    pub fn new() -> Self {
        Renderer
    }

    pub fn render(&self, _framebuffer: &[u8]) {
        // Rendering is now handled by the free functions above.
    }
}
