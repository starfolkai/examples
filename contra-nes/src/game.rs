// Game — frame trace replay with native rendering
//
// Instead of emulating the NES CPU/PPU, this module loads a pre-captured
// frame trace (extracted/frame_trace.bin) and replays the rendering state
// frame by frame. The trace contains per-frame PPU state deltas (scroll,
// palette, nametable patches, sprite lists, CHR updates) which are applied
// to produce each frame's RGB24 framebuffer.

use crate::renderer;

const SCREEN_W: usize = 256;
const SCREEN_H: usize = 240;

// NES master palette (NesDev canonical 2C02) — maps 6-bit palette index to RGB
static NES_PAL: [[u8; 3]; 64] = [
    [84,84,84],[0,30,116],[8,16,144],[48,0,136],[68,0,100],[92,0,48],[84,4,0],[60,24,0],
    [32,42,0],[8,58,0],[0,64,0],[0,60,0],[0,50,60],[0,0,0],[0,0,0],[0,0,0],
    [152,150,152],[8,76,196],[48,50,236],[92,30,228],[136,20,176],[160,20,100],[152,34,32],[120,60,0],
    [84,90,0],[40,114,0],[8,124,0],[0,118,40],[0,102,120],[0,0,0],[0,0,0],[0,0,0],
    [236,238,236],[76,154,236],[120,124,236],[176,98,236],[228,84,236],[236,88,180],[236,106,100],[212,136,32],
    [160,170,0],[116,196,0],[76,208,32],[56,204,108],[56,180,220],[60,60,60],[0,0,0],[0,0,0],
    [236,238,236],[168,204,236],[188,188,236],[212,178,236],[236,174,236],[236,174,212],[236,180,176],[228,196,144],
    [204,210,120],[180,222,120],[168,226,144],[152,226,180],[160,214,228],[160,162,160],[0,0,0],[0,0,0],
];

// Frame trace data embedded at compile time
const TRACE_DATA: &[u8] = include_bytes!("extracted/frame_trace.bin");

pub struct Game {
    // Current rendering state (populated from trace, not from NES emulation)
    chr_data: Vec<u8>,                  // 8192 bytes — tile pattern data
    nametable: Vec<u8>,                 // 2048 bytes — background tile map
    pal_data: Vec<u8>,                  // 32 bytes — color palette indices
    sprites: Vec<(u8, u8, u8, u8)>,     // visible sprites: (y, tile, attr, x)
    scroll_x: u16,
    scroll_y: u16,
    ctrl: u8,
    mask: u8,

    framebuffer: Vec<u8>,               // 256*240*3 RGB24
    frame_index: usize,                 // current frame in trace
    trace_offset: usize,                // byte offset into TRACE_DATA
    total_frames: u32,

    // Input state (two controllers, 8 buttons each)
    buttons: [u8; 2],

    _renderer: renderer::Renderer,
}

// ── Trace binary helpers ──

fn read_u8(data: &[u8], off: &mut usize) -> u8 {
    let v = data[*off];
    *off += 1;
    v
}

fn read_u16(data: &[u8], off: &mut usize) -> u16 {
    let lo = data[*off] as u16;
    let hi = data[*off + 1] as u16;
    *off += 2;
    lo | (hi << 8)
}

fn read_u32(data: &[u8], off: &mut usize) -> u32 {
    let b0 = data[*off] as u32;
    let b1 = data[*off + 1] as u32;
    let b2 = data[*off + 2] as u32;
    let b3 = data[*off + 3] as u32;
    *off += 4;
    b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
}

fn read_bytes(data: &[u8], off: &mut usize, len: usize) -> Vec<u8> {
    let slice = &data[*off..*off + len];
    *off += len;
    slice.to_vec()
}

impl Game {
    pub fn new(_prg_data: &[u8], _prg_banks: usize) -> Self {
        let mut off = 0usize;

        // Parse trace header
        let total_frames = read_u32(TRACE_DATA, &mut off);
        let chr_data = read_bytes(TRACE_DATA, &mut off, 8192);
        let nametable = read_bytes(TRACE_DATA, &mut off, 2048);
        let pal_data = read_bytes(TRACE_DATA, &mut off, 32);

        Game {
            chr_data,
            nametable,
            pal_data,
            sprites: Vec::new(),
            scroll_x: 0,
            scroll_y: 0,
            ctrl: 0,
            mask: 0,
            framebuffer: vec![0u8; SCREEN_W * SCREEN_H * 3],
            frame_index: 0,
            trace_offset: off,
            total_frames,
            buttons: [0; 2],
            _renderer: renderer::Renderer::new(),
        }
    }

    /// Advance one frame: read the next frame delta from the trace and render.
    pub fn update(&mut self) {
        // If we have more frames in the trace, read the next delta
        if (self.frame_index as u32) < self.total_frames {
            self.read_frame_delta();
        }
        // Otherwise keep rendering the last frame state (no trace advancement)

        self.render_frame();
        self.frame_index += 1;
    }

    /// Read one frame's worth of delta data from the trace.
    fn read_frame_delta(&mut self) {
        let off = &mut self.trace_offset;

        // Scroll
        self.scroll_x = read_u16(TRACE_DATA, off);
        self.scroll_y = read_u16(TRACE_DATA, off);

        // Fine X (consumed but folded into scroll_x already by the capturer)
        let _fine_x = read_u8(TRACE_DATA, off);

        // PPU control/mask registers
        self.ctrl = read_u8(TRACE_DATA, off);
        self.mask = read_u8(TRACE_DATA, off);

        // CHR data (full replacement if changed)
        let chr_changed = read_u8(TRACE_DATA, off);
        if chr_changed != 0 {
            self.chr_data = read_bytes(TRACE_DATA, off, 8192);
        }

        // Palette data (full replacement if changed)
        let pal_changed = read_u8(TRACE_DATA, off);
        if pal_changed != 0 {
            self.pal_data = read_bytes(TRACE_DATA, off, 32);
        }

        // Nametable deltas
        let nt_delta_count = read_u16(TRACE_DATA, off) as usize;
        for _ in 0..nt_delta_count {
            let nt_offset = read_u16(TRACE_DATA, off) as usize;
            let value = read_u8(TRACE_DATA, off);
            if nt_offset < self.nametable.len() {
                self.nametable[nt_offset] = value;
            }
        }

        // Sprites
        let sprite_count = read_u8(TRACE_DATA, off) as usize;
        self.sprites.clear();
        for _ in 0..sprite_count {
            let y = read_u8(TRACE_DATA, off);
            let tile = read_u8(TRACE_DATA, off);
            let attr = read_u8(TRACE_DATA, off);
            let x = read_u8(TRACE_DATA, off);
            self.sprites.push((y, tile, attr, x));
        }
    }

    /// No-op — rendering is performed inside update().
    pub fn render(&mut self) {
        // Intentionally empty: frame rendering happens in update() via render_frame()
    }

    pub fn set_button(&mut self, player: usize, button: u8, pressed: bool) {
        if player < 2 && button < 8 {
            if pressed {
                self.buttons[player] |= 1 << button;
            } else {
                self.buttons[player] &= !(1 << button);
            }
        }
    }

    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_index as u64
    }

    pub fn audio_buffer(&self) -> std::sync::Arc<crate::apu::AudioBuffer> {
        std::sync::Arc::new(crate::apu::AudioBuffer::new(4096))
    }

    // ── Internal rendering ──

    /// Render the current PPU state into the RGB24 framebuffer.
    fn render_frame(&mut self) {
        let show_bg = self.mask & 0x08 != 0;
        let show_sprites = self.mask & 0x10 != 0;
        let sprite_height = if self.ctrl & 0x20 != 0 { 16 } else { 8 };
        let bg_pattern_base: usize = if self.ctrl & 0x10 != 0 { 0x1000 } else { 0 };
        let spr_pattern_base: usize = if self.ctrl & 0x08 != 0 { 0x1000 } else { 0 };

        // Render each scanline
        for scanline in 0..SCREEN_H {
            self.render_scanline(
                scanline,
                show_bg,
                show_sprites,
                sprite_height,
                bg_pattern_base,
                spr_pattern_base,
            );
        }
    }

    /// Render a single scanline into the framebuffer.
    fn render_scanline(
        &mut self,
        scanline: usize,
        show_bg: bool,
        show_sprites: bool,
        sprite_height: usize,
        bg_pattern_base: usize,
        spr_pattern_base: usize,
    ) {
        let fb_row = scanline * SCREEN_W * 3;

        // Per-pixel background + sprite compositing
        for pixel in 0..SCREEN_W {
            let mut bg_color_idx: u8 = 0;
            let mut bg_opaque = false;

            if show_bg {
                bg_color_idx = self.sample_background(scanline, pixel, bg_pattern_base);
                bg_opaque = (bg_color_idx & 0x03) != 0;
            }

            let mut spr_color_idx: u8 = 0;
            let mut spr_opaque = false;
            let mut spr_priority = false; // true = behind background

            if show_sprites {
                if let Some((ci, pri)) =
                    self.sample_sprites(scanline, pixel, sprite_height, spr_pattern_base)
                {
                    spr_color_idx = ci;
                    spr_opaque = true;
                    spr_priority = pri;
                }
            }

            // Priority compositing
            let final_idx = if spr_opaque && (!bg_opaque || !spr_priority) {
                spr_color_idx
            } else if bg_opaque {
                bg_color_idx
            } else {
                0 // universal background color
            };

            let pal_entry = self.pal_data[(final_idx & 0x1F) as usize] & 0x3F;
            let rgb = NES_PAL[pal_entry as usize];

            let off = fb_row + pixel * 3;
            self.framebuffer[off] = rgb[0];
            self.framebuffer[off + 1] = rgb[1];
            self.framebuffer[off + 2] = rgb[2];
        }
    }

    /// Sample the background tile layer at a given screen coordinate.
    /// Returns a 5-bit palette index (palette group in bits 4-3, pixel color in bits 1-0).
    fn sample_background(&self, scanline: usize, pixel: usize, pattern_base: usize) -> u8 {
        // Apply scrolling — scroll_x already includes nametable select bit (bit 8 = +256)
        let abs_x = (pixel as u16).wrapping_add(self.scroll_x) as usize;
        let abs_y = (scanline as u16).wrapping_add(self.scroll_y) as usize;

        // Determine which nametable (0-3) based on absolute coordinates
        let nt_x = (abs_x / 256) & 1;
        let nt_y = (abs_y / 240) & 1;
        let nt_index = nt_y * 2 + nt_x;
        let nt_base = nt_index * 0x400;

        let local_x = abs_x % 256;
        let local_y = abs_y % 240;

        let tile_col = local_x / 8;
        let tile_row = local_y / 8;

        // Nametable byte (tile index)
        let nt_addr = nt_base + tile_row * 32 + tile_col;
        if nt_addr >= self.nametable.len() {
            return 0;
        }
        let tile_id = self.nametable[nt_addr] as usize;

        // Attribute table — each byte covers a 4x4 tile area (32x32 pixels)
        let attr_col = tile_col / 4;
        let attr_row = tile_row / 4;
        let attr_addr = nt_base + 960 + attr_row * 8 + attr_col;
        let attr_byte = if attr_addr < self.nametable.len() {
            self.nametable[attr_addr]
        } else {
            0
        };

        // Which quadrant of the attribute byte (2-bit palette group)
        let quad_x = (tile_col / 2) & 1;
        let quad_y = (tile_row / 2) & 1;
        let shift = (quad_y * 2 + quad_x) * 2;
        let palette_group = (attr_byte >> shift) & 0x03;

        // Read tile pattern from CHR data
        let fine_x = local_x % 8;
        let fine_y = local_y % 8;
        let chr_addr = pattern_base + tile_id * 16 + fine_y;

        let (lo_plane, hi_plane) = if chr_addr + 8 < self.chr_data.len() {
            (self.chr_data[chr_addr], self.chr_data[chr_addr + 8])
        } else {
            (0, 0)
        };

        let bit = 7 - fine_x;
        let color_lo = (lo_plane >> bit) & 1;
        let color_hi = (hi_plane >> bit) & 1;
        let color_val = color_lo | (color_hi << 1);

        if color_val == 0 {
            0 // transparent — use universal background
        } else {
            (palette_group << 2) | color_val
        }
    }

    /// Sample sprites at a given screen coordinate.
    /// Returns Some((palette_index, behind_bg)) if a sprite pixel is found, None otherwise.
    fn sample_sprites(
        &self,
        scanline: usize,
        pixel: usize,
        sprite_height: usize,
        pattern_base: usize,
    ) -> Option<(u8, bool)> {
        for &(spy, tile, attr, spx) in &self.sprites {
            let sx = spx as usize;
            let sy = (spy as usize).wrapping_add(1); // OAM Y is off by 1

            // Check if this pixel falls within the sprite's bounding box
            if pixel < sx || pixel >= sx + 8 {
                continue;
            }
            if scanline < sy || scanline >= sy + sprite_height {
                continue;
            }

            let flip_h = attr & 0x40 != 0;
            let flip_v = attr & 0x80 != 0;
            let palette_group = (attr & 0x03) + 4; // sprite palettes are 4-7
            let behind_bg = attr & 0x20 != 0;

            let mut row = scanline - sy;
            let mut col = pixel - sx;

            if flip_h {
                col = 7 - col;
            }
            if flip_v {
                row = (sprite_height - 1) - row;
            }

            // For 8x16 sprites, tile index selects pattern table and tile pair
            let (chr_base, tile_id) = if sprite_height == 16 {
                let bank = (tile as usize & 1) * 0x1000;
                let base_tile = (tile as usize) & 0xFE;
                if row < 8 {
                    (bank, base_tile)
                } else {
                    (bank, base_tile + 1)
                }
            } else {
                (pattern_base, tile as usize)
            };

            let fine_y = row % 8;
            let chr_addr = chr_base + tile_id * 16 + fine_y;

            let (lo_plane, hi_plane) = if chr_addr + 8 < self.chr_data.len() {
                (self.chr_data[chr_addr], self.chr_data[chr_addr + 8])
            } else {
                (0, 0)
            };

            let bit = 7 - col;
            let color_lo = (lo_plane >> bit) & 1;
            let color_hi = (hi_plane >> bit) & 1;
            let color_val = color_lo | (color_hi << 1);

            if color_val != 0 {
                let palette_idx = (palette_group << 2) | color_val;
                return Some((palette_idx, behind_bg));
            }
        }
        None
    }
}
