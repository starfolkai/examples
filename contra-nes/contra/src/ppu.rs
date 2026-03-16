// NES PPU (2C02) — scanline-accurate rendering
//
// 262 scanlines per frame. 341 dots per scanline.
// Visible: scanlines 0-239. VBlank: 241-260. Pre-render: 261.
// Each dot is 1 PPU cycle. 3 PPU cycles = 1 CPU cycle.

use crate::cartridge::Cartridge;

const SCREEN_W: usize = 256;
const SCREEN_H: usize = 240;

// NES master palette (NesDev canonical 2C02)
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

pub struct Ppu {
    pub nt_ram: [u8; 2048],
    pub palette: [u8; 32],
    pub oam: [u8; 256],
    pub ctrl: u8,
    pub mask: u8,
    pub status: u8,
    pub oam_addr: u8,
    pub v: u16,
    pub t: u16,
    pub x: u8,
    pub w: bool,
    pub read_buf: u8,
    pub open_bus: u8,
    pub scanline: i32,
    pub dot: u32,
    pub frame_count: u64,
    pub odd_frame: bool,
    bg_lo_shift: u16,
    bg_hi_shift: u16,
    at_lo_shift: u16,
    at_hi_shift: u16,
    nt_byte: u8,
    at_byte: u8,
    bg_lo: u8,
    bg_hi: u8,
    sprite_count: usize,
    sp_line: [u8; SCREEN_W],
    sp_line_has_sprites: bool,
    pub framebuffer: [u32; SCREEN_W * SCREEN_H],
    pal_cache: [u32; 32],
    pub nmi_output: bool,
    pub nmi_occurred: bool,
    pub nmi_line: bool,
    nmi_delay: u8,
}

impl Ppu {
    pub fn new() -> Self {
        let mut ppu = Ppu {
            nt_ram: [0; 2048],
            palette: [0; 32],
            oam: [0; 256],
            ctrl: 0, mask: 0, status: 0, oam_addr: 0,
            v: 0, t: 0, x: 0, w: false,
            read_buf: 0, open_bus: 0,
            scanline: 0, dot: 0, frame_count: 0, odd_frame: false,
            bg_lo_shift: 0, bg_hi_shift: 0,
            at_lo_shift: 0, at_hi_shift: 0,
            nt_byte: 0, at_byte: 0, bg_lo: 0, bg_hi: 0,
            sprite_count: 0,
            sp_line: [0; SCREEN_W],
            sp_line_has_sprites: false,
            framebuffer: [0; SCREEN_W * SCREEN_H],
            pal_cache: [0; 32],
            nmi_output: false, nmi_occurred: false, nmi_line: false,
            nmi_delay: 0,
        };
        ppu.rebuild_pal_cache();
        ppu
    }

    #[inline(always)]
    fn rebuild_pal_cache(&mut self) {
        for i in 0..32 {
            let c = (self.palette[i] & 0x3F) as usize;
            let rgb = unsafe { NES_PAL.get_unchecked(c) };
            self.pal_cache[i] = (rgb[0] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[2] as u32;
        }
    }

    // ── Register reads ──

    pub fn read_register(&mut self, addr: u16, cart: &Cartridge) -> u8 {
        match addr & 7 {
            2 => { // PPUSTATUS
                let mut val = self.status & 0xE0;
                val |= self.open_bus & 0x1F;
                self.nmi_occurred = false;
                self.status &= !0x80;
                self.nmi_change();
                self.w = false;
                self.open_bus = val;
                val
            }
            4 => { // OAMDATA
                let val = self.oam[self.oam_addr as usize];
                self.open_bus = val;
                val
            }
            7 => { // PPUDATA
                let mut val = self.vram_read(self.v, cart);
                // Buffered reads for non-palette addresses
                if self.v & 0x3FFF < 0x3F00 {
                    let buf = self.read_buf;
                    self.read_buf = val;
                    val = buf;
                } else {
                    self.read_buf = self.vram_read(self.v - 0x1000, cart);
                }
                self.v = self.v.wrapping_add(if self.ctrl & 4 != 0 { 32 } else { 1 });
                self.open_bus = val;
                val
            }
            _ => self.open_bus,
        }
    }

    pub fn write_register(&mut self, addr: u16, val: u8, cart: &mut Cartridge) {
        self.open_bus = val;
        match addr & 7 {
            0 => { // PPUCTRL
                self.ctrl = val;
                self.nmi_output = val & 0x80 != 0;
                self.nmi_change();
                // t: ...GH.. ........ <- val: ......GH
                self.t = (self.t & 0xF3FF) | ((val as u16 & 3) << 10);
            }
            1 => { self.mask = val; }
            3 => { self.oam_addr = val; }
            4 => {
                self.oam[self.oam_addr as usize] = val;
                self.oam_addr = self.oam_addr.wrapping_add(1);
            }
            5 => { // PPUSCROLL
                if !self.w {
                    // First write: X scroll
                    self.t = (self.t & 0xFFE0) | ((val as u16) >> 3);
                    self.x = val & 7;
                } else {
                    // Second write: Y scroll
                    self.t = (self.t & 0x8C1F) | ((val as u16 & 7) << 12)
                        | ((val as u16 >> 3) << 5);
                }
                self.w = !self.w;
            }
            6 => { // PPUADDR
                if !self.w {
                    self.t = (self.t & 0x00FF) | ((val as u16 & 0x3F) << 8);
                } else {
                    self.t = (self.t & 0xFF00) | val as u16;
                    self.v = self.t;
                }
                self.w = !self.w;
            }
            7 => { // PPUDATA
                self.vram_write(self.v, val, cart);
                self.v = self.v.wrapping_add(if self.ctrl & 4 != 0 { 32 } else { 1 });
            }
            _ => {}
        }
    }

    pub fn write_oam_dma(&mut self, data: &[u8; 256]) {
        if self.oam_addr == 0 {
            // Fast path: aligned DMA (most common) — single memcpy
            self.oam.copy_from_slice(data);
        } else {
            // Unaligned DMA: wrap around OAM
            let start = self.oam_addr as usize;
            let first = 256 - start;
            self.oam[start..].copy_from_slice(&data[..first]);
            self.oam[..start].copy_from_slice(&data[first..]);
            // oam_addr wraps back to same value after 256 writes
        }
    }

    // ── VRAM access ──

    fn vram_read(&self, addr: u16, cart: &Cartridge) -> u8 {
        let a = addr & 0x3FFF;
        if a < 0x2000 {
            cart.read_chr(a)
        } else if a < 0x3F00 {
            let idx = cart.mirror_nt(a & 0x2FFF);
            unsafe { *self.nt_ram.get_unchecked(idx) }
        } else {
            let mut pa = (a - 0x3F00) as usize & 0x1F;
            if pa == 0x10 || pa == 0x14 || pa == 0x18 || pa == 0x1C { pa &= 0x0F; }
            self.palette[pa]
        }
    }

    fn vram_write(&mut self, addr: u16, val: u8, cart: &mut Cartridge) {
        let a = addr & 0x3FFF;
        if a < 0x2000 {
            cart.write_chr(a, val);
        } else if a < 0x3F00 {
            let idx = cart.mirror_nt(a & 0x2FFF);
            self.nt_ram[idx] = val;
        } else {
            let mut pa = (a - 0x3F00) as usize & 0x1F;
            if pa == 0x10 || pa == 0x14 || pa == 0x18 || pa == 0x1C { pa &= 0x0F; }
            self.palette[pa] = val;
            self.rebuild_pal_cache();
        }
    }

    fn nmi_change(&mut self) {
        let nmi = self.nmi_output && self.nmi_occurred;
        if nmi && !self.nmi_line {
            self.nmi_delay = 15; // slight delay
        }
        self.nmi_line = nmi;
    }

    // ── Rendering helpers ──

    #[inline(always)]
    fn fetch_nt_byte(&mut self, cart: &Cartridge) {
        let addr = 0x2000 | (self.v & 0x0FFF);
        let idx = cart.mirror_nt(addr);
        self.nt_byte = unsafe { *self.nt_ram.get_unchecked(idx) };
    }

    #[inline(always)]
    fn fetch_at_byte(&mut self, cart: &Cartridge) {
        let v = self.v;
        let addr = 0x23C0 | (v & 0x0C00) | ((v >> 4) & 0x38) | ((v >> 2) & 7);
        let shift = ((v >> 4) & 4) | (v & 2);
        let idx = cart.mirror_nt(addr);
        self.at_byte = (unsafe { *self.nt_ram.get_unchecked(idx) } >> shift) & 3;
    }

    #[inline(always)]
    fn fetch_tile_lo(&mut self, cart: &Cartridge) {
        let fine_y = (self.v >> 12) & 7;
        let base = if self.ctrl & 0x10 != 0 { 0x1000u16 } else { 0 };
        let addr = base + (self.nt_byte as u16) * 16 + fine_y;
        self.bg_lo = cart.read_chr(addr);
    }

    #[inline(always)]
    fn fetch_tile_hi(&mut self, cart: &Cartridge) {
        let fine_y = (self.v >> 12) & 7;
        let base = if self.ctrl & 0x10 != 0 { 0x1000u16 } else { 0 };
        let addr = base + (self.nt_byte as u16) * 16 + fine_y + 8;
        self.bg_hi = cart.read_chr(addr);
    }

    #[inline(always)]
    fn load_bg_shifters(&mut self) {
        self.bg_lo_shift = (self.bg_lo_shift & 0xFF00) | self.bg_lo as u16;
        self.bg_hi_shift = (self.bg_hi_shift & 0xFF00) | self.bg_hi as u16;
        self.at_lo_shift = (self.at_lo_shift & 0xFF00) | if self.at_byte & 1 != 0 { 0xFF } else { 0x00 };
        self.at_hi_shift = (self.at_hi_shift & 0xFF00) | if self.at_byte & 2 != 0 { 0xFF } else { 0x00 };
    }

    #[inline(always)]
    fn shift_bg(&mut self) {
        self.bg_lo_shift <<= 1;
        self.bg_hi_shift <<= 1;
        self.at_lo_shift <<= 1;
        self.at_hi_shift <<= 1;
    }

    #[inline(always)]
    fn increment_x(&mut self) {
        if self.v & 0x001F == 31 {
            self.v &= !0x001F;
            self.v ^= 0x0400; // switch horizontal nametable
        } else {
            self.v += 1;
        }
    }

    #[inline(always)]
    fn increment_y(&mut self) {
        if (self.v & 0x7000) != 0x7000 {
            self.v += 0x1000;
        } else {
            self.v &= !0x7000;
            let mut y = (self.v & 0x03E0) >> 5;
            if y == 29 {
                y = 0;
                self.v ^= 0x0800;
            } else if y == 31 {
                y = 0;
            } else {
                y += 1;
            }
            self.v = (self.v & !0x03E0) | (y << 5);
        }
    }

    #[inline(always)]
    fn copy_x(&mut self) {
        self.v = (self.v & 0xFBE0) | (self.t & 0x041F);
    }

    #[inline(always)]
    fn copy_y(&mut self) {
        self.v = (self.v & 0x841F) | (self.t & 0x7BE0);
    }

    // Sprite evaluation + pre-render scanline buffer
    fn evaluate_sprites(&mut self, cart: &Cartridge) {
        let tall = self.ctrl & 0x20 != 0;
        let h: i32 = if tall { 16 } else { 8 };
        self.sprite_count = 0;

        // Clear only if previous scanline had sprites
        if self.sp_line_has_sprites {
            self.sp_line = [0; SCREEN_W];
        }
        self.sp_line_has_sprites = false;

        let scanline = self.scanline;

        for i in 0..64usize {
            let base_idx = i * 4;
            let y = unsafe { *self.oam.get_unchecked(base_idx) } as i32;
            let row = scanline - y;
            if row < 0 || row >= h { continue; }
            if self.sprite_count >= 8 {
                self.status |= 0x20;
                break;
            }

            let tile_idx = unsafe { *self.oam.get_unchecked(base_idx + 1) };
            let attr = unsafe { *self.oam.get_unchecked(base_idx + 2) };
            let sx = unsafe { *self.oam.get_unchecked(base_idx + 3) } as usize;
            let flip_h = attr & 0x40 != 0;
            let flip_v = attr & 0x80 != 0;
            let palette = (attr & 3) + 4;
            let priority = (attr >> 5) & 1;

            let mut row = row as u16;
            if flip_v { row = h as u16 - 1 - row; }

            let (base, tile) = if tall {
                let base = if tile_idx & 1 != 0 { 0x1000u16 } else { 0u16 };
                let tile = tile_idx & 0xFE;
                if row >= 8 { (base, tile + 1) } else { (base, tile) }
            } else {
                let base = if self.ctrl & 0x08 != 0 { 0x1000u16 } else { 0u16 };
                (base, tile_idx)
            };

            let addr = base + tile as u16 * 16 + (row & 7);
            let mut lo = cart.read_chr(addr);
            let mut hi = cart.read_chr(addr + 8);

            // Skip fully transparent sprite rows
            if lo | hi == 0 {
                self.sprite_count += 1;
                continue;
            }

            if flip_h { lo = lo.reverse_bits(); hi = hi.reverse_bits(); }

            let zero_flag: u8 = if i == 0 { 0x40 } else { 0 };
            let prio_bits: u8 = priority << 5;
            let pal_bits: u8 = palette << 2;
            let combined = pal_bits | prio_bits | zero_flag;

            // Pre-render into scanline buffer (first sprite wins)
            for dx in 0..8u8 {
                let px = sx + dx as usize;
                if px >= SCREEN_W { break; }
                // Skip if another sprite already occupies this pixel
                let existing = unsafe { *self.sp_line.get_unchecked(px) };
                if existing & 0x03 != 0 { continue; }

                let bit = 7 - dx;
                let pixel = ((lo >> bit) & 1) | (((hi >> bit) & 1) << 1);
                if pixel == 0 { continue; }

                unsafe {
                    *self.sp_line.get_unchecked_mut(px) = pixel | combined;
                }
            }

            self.sprite_count += 1;
        }

        self.sp_line_has_sprites = self.sprite_count > 0;
    }

    // Prefetch 2 tiles (dots 321-336) — direct CHR reads
    #[inline(always)]
    fn prefetch_two_tiles(&mut self, cart: &Cartridge) {
        let fine_y = (self.v >> 12) & 7;
        let bg_base = if self.ctrl & 0x10 != 0 { 0x1000u16 } else { 0 };
        // Tile 1 (dots 321-328)
        self.fetch_nt_byte(cart);
        self.fetch_at_byte(cart);
        let chr_addr = bg_base + (self.nt_byte as u16) * 16 + fine_y;
        self.bg_lo = cart.read_chr(chr_addr);
        self.bg_hi = cart.read_chr(chr_addr + 8);
        self.load_bg_shifters();
        self.increment_x();
        self.bg_lo_shift <<= 8;
        self.bg_hi_shift <<= 8;
        self.at_lo_shift <<= 8;
        self.at_hi_shift <<= 8;
        // Tile 2 (dots 329-336)
        self.fetch_nt_byte(cart);
        self.fetch_at_byte(cart);
        let chr_addr = bg_base + (self.nt_byte as u16) * 16 + fine_y;
        self.bg_lo = cart.read_chr(chr_addr);
        self.bg_hi = cart.read_chr(chr_addr + 8);
        self.load_bg_shifters();
        self.increment_x();
        self.bg_lo_shift <<= 8;
        self.bg_hi_shift <<= 8;
        self.at_lo_shift <<= 8;
        self.at_hi_shift <<= 8;
    }

    // ── Scanline-batched rendering ──
    // Called once per scanline from nes.rs. Processes all 341 dots at once.
    // This replaces ~89K individual tick() calls with ~262 batch calls per frame.

    pub fn finish_scanline(&mut self, cart: &Cartridge) -> bool {
        let mut nmi = false;
        let sl = self.scanline;
        let rendering = self.mask & 0x18 != 0;

        // Handle NMI delay (max 15 ticks, always resolves within one scanline)
        if self.nmi_delay > 0 {
            self.nmi_delay = 0;
            if self.nmi_output && self.nmi_occurred {
                nmi = true;
            }
        }

        if sl < 240 {
            // ── Visible scanline (0-239) ──
            if rendering {
                // Render all 256 pixels in a tight loop
                self.render_scanline_pixels(cart);
                // Dot 256: increment Y
                self.increment_y();
                // Dot 257: copy X + evaluate sprites for next scanline
                self.copy_x();
                self.evaluate_sprites(cart);
                // Dots 321-336: prefetch first 2 tiles of next scanline (unrolled)
                self.prefetch_two_tiles(cart);
            }
        } else if sl == 240 {
            // Post-render scanline — idle
        } else if sl == 241 {
            // ── VBlank start ──
            self.status |= 0x80;
            self.nmi_occurred = true;
            self.nmi_change();
        } else if sl >= 242 && sl <= 260 {
            // VBlank interior — idle
        } else if sl == 261 {
            // ── Pre-render scanline ──
            // Clear flags
            self.status &= !0xE0;
            self.nmi_occurred = false;
            self.nmi_change();

            if rendering {
                // Copy Y (done at dots 280-304, effect is idempotent)
                self.copy_y();
                // Prefetch tiles for scanline 0 (unrolled)
                self.prefetch_two_tiles(cart);
            }
        }

        // Advance to next scanline
        self.dot = 0;
        self.scanline = sl + 1;
        if self.scanline > 261 {
            self.scanline = 0;
            self.odd_frame = !self.odd_frame;
            self.frame_count += 1;
        }

        nmi
    }

    // Render all 256 pixels of a visible scanline.
    #[inline(always)]
    fn render_scanline_pixels(&mut self, cart: &Cartridge) {
        let py = self.scanline as usize;
        let show_bg = self.mask & 0x08 != 0;
        let show_sp = self.mask & 0x10 != 0;
        let show_bg_left = self.mask & 0x02 != 0;
        let show_sp_left = self.mask & 0x04 != 0;
        let has_sprites = self.sp_line_has_sprites;
        let fine_x = self.x;
        let fb_row = py * SCREEN_W;

        if !has_sprites || !show_sp {
            self.render_scanline_bg_only(cart, show_bg, show_bg_left, fine_x, fb_row);
        } else {
            self.render_scanline_full(cart, show_bg, show_sp, show_bg_left, show_sp_left, fine_x, fb_row);
        }
    }

    // Background-only scanline — no sprite checks, no priority mux
    #[inline(always)]
    fn render_scanline_bg_only(&mut self, cart: &Cartridge, show_bg: bool, show_bg_left: bool, fine_x: u8, fb_row: usize) {
        let bg_color = unsafe { *self.pal_cache.get_unchecked(0) };

        for tile in 0u32..32 {
            let tile_px = (tile * 8) as usize;
            self.fetch_nt_byte(cart);
            self.fetch_at_byte(cart);
            self.fetch_tile_lo(cart);
            self.fetch_tile_hi(cart);

            for sub in 0u8..8 {
                let px = tile_px + sub as usize;
                let color = if show_bg && (px >= 8 || show_bg_left) {
                    let bit = 15 - fine_x;
                    let p = ((self.bg_lo_shift >> bit) & 1) as u8
                        | (((self.bg_hi_shift >> bit) & 1) << 1) as u8;
                    if p != 0 {
                        let a = ((self.at_lo_shift >> bit) & 1) as u8
                            | (((self.at_hi_shift >> bit) & 1) << 1) as u8;
                        unsafe { *self.pal_cache.get_unchecked(a as usize * 4 + p as usize) }
                    } else {
                        bg_color
                    }
                } else {
                    bg_color
                };

                unsafe { *self.framebuffer.get_unchecked_mut(fb_row + px) = color; }
                self.bg_lo_shift <<= 1;
                self.bg_hi_shift <<= 1;
                self.at_lo_shift <<= 1;
                self.at_hi_shift <<= 1;
            }
            self.load_bg_shifters();
            self.increment_x();
        }
    }

    // Full scanline with sprite priority mux
    #[inline(always)]
    fn render_scanline_full(&mut self, cart: &Cartridge, show_bg: bool, show_sp: bool, show_bg_left: bool, show_sp_left: bool, fine_x: u8, fb_row: usize) {
        for tile in 0u32..32 {
            let tile_px = (tile * 8) as usize;
            self.fetch_nt_byte(cart);
            self.fetch_at_byte(cart);
            self.fetch_tile_lo(cart);
            self.fetch_tile_hi(cart);

            for sub in 0u8..8 {
                let px = tile_px + sub as usize;

                let (bg_pixel, bg_pal) = if show_bg && (px >= 8 || show_bg_left) {
                    let bit = 15 - fine_x;
                    let p = ((self.bg_lo_shift >> bit) & 1) as u8
                        | (((self.bg_hi_shift >> bit) & 1) << 1) as u8;
                    let a = ((self.at_lo_shift >> bit) & 1) as u8
                        | (((self.at_hi_shift >> bit) & 1) << 1) as u8;
                    (p, a)
                } else {
                    (0, 0)
                };

                let sp_data = if show_sp && (px >= 8 || show_sp_left) {
                    unsafe { *self.sp_line.get_unchecked(px) }
                } else {
                    0
                };
                let sp_pixel = sp_data & 0x03;

                let pal_addr = match (bg_pixel != 0, sp_pixel != 0) {
                    (false, false) => 0usize,
                    (false, true) => ((sp_data >> 2) & 7) as usize * 4 + sp_pixel as usize,
                    (true, false) => bg_pal as usize * 4 + bg_pixel as usize,
                    (true, true) => {
                        if sp_data & 0x40 != 0 && px < 255 {
                            self.status |= 0x40;
                        }
                        if (sp_data >> 5) & 1 == 0 {
                            ((sp_data >> 2) & 7) as usize * 4 + sp_pixel as usize
                        } else {
                            bg_pal as usize * 4 + bg_pixel as usize
                        }
                    }
                };

                unsafe {
                    *self.framebuffer.get_unchecked_mut(fb_row + px) = *self.pal_cache.get_unchecked(pal_addr);
                }
                self.bg_lo_shift <<= 1; self.bg_hi_shift <<= 1;
                self.at_lo_shift <<= 1; self.at_hi_shift <<= 1;
            }
            self.load_bg_shifters();
            self.increment_x();
        }
    }


    /// Minimal scanline — no pixel rendering, only timing/NMI/flags
    pub fn finish_scanline_minimal(&mut self, cart: &Cartridge) -> bool {
        let mut nmi = false;
        let sl = self.scanline;

        if self.nmi_delay > 0 {
            self.nmi_delay = 0;
            if self.nmi_output && self.nmi_occurred {
                nmi = true;
            }
        }

        if sl == 241 {
            self.status |= 0x80;
            self.nmi_occurred = true;
            self.nmi_change();
        } else if sl == 261 {
            self.status &= !0xE0;
            self.nmi_occurred = false;
            self.nmi_change();
        }

        self.dot = 0;
        self.scanline = sl + 1;
        if self.scanline > 261 {
            self.scanline = 0;
            self.odd_frame = !self.odd_frame;
            self.frame_count += 1;
        }

        nmi
    }

    // ── Per-dot tick (used by tests that include PPU source directly) ──

    #[allow(dead_code)]
    #[inline(always)]
    pub fn tick(&mut self, cart: &Cartridge) -> bool {
        let mut nmi_triggered = false;

        if self.nmi_delay > 0 {
            self.nmi_delay -= 1;
            if self.nmi_delay == 0 && self.nmi_output && self.nmi_occurred {
                nmi_triggered = true;
            }
        }

        let dot = self.dot;
        let scanline = self.scanline;

        // Fast path: most dots on non-render scanlines (241-260) do nothing
        if scanline >= 240 && scanline < 261 {
            // VBlank region — only check NMI trigger
            if scanline == 241 && dot == 1 {
                self.status |= 0x80;
                self.nmi_occurred = true;
                self.nmi_change();
            }
        } else {
            let rendering = self.mask & 0x18 != 0;
            let visible_line = scanline < 240;
            let pre_line = scanline == 261;

            if rendering {
                // Visible pixel output (dots 1-256 on scanlines 0-239)
                if visible_line && dot >= 1 && dot <= 256 {
                    self.render_pixel();
                }

                // Background fetch (dots 1-256 and 321-336)
                let is_fetch = (dot >= 1 && dot <= 256) || (dot >= 321 && dot <= 336);
                if (visible_line || pre_line) && is_fetch {
                    self.shift_bg();
                    match dot & 7 {
                        1 => self.fetch_nt_byte(cart),
                        3 => self.fetch_at_byte(cart),
                        5 => self.fetch_tile_lo(cart),
                        7 => {
                            self.fetch_tile_hi(cart);
                            self.load_bg_shifters();
                            self.increment_x();
                        }
                        _ => {}
                    }
                }

                if visible_line || pre_line {
                    if dot == 256 {
                        self.increment_y();
                    } else if dot == 257 {
                        self.copy_x();
                        if visible_line {
                            self.evaluate_sprites(cart);
                        }
                    }
                }

                // Pre-render: copy Y at dots 280-304
                if pre_line && dot >= 280 && dot <= 304 {
                    self.copy_y();
                }
            }

            if pre_line && dot == 1 {
                self.status &= !0xE0;
                self.nmi_occurred = false;
                self.nmi_change();
            }
        }

        // ── Advance dot/scanline ──
        self.dot = dot + 1;

        if dot >= 340 {
            // Odd frame skip
            let rendering = self.mask & 0x18 != 0;
            if rendering && self.odd_frame && scanline == 261 && dot == 340 {
                self.dot = 0;
                self.scanline = 0;
                self.odd_frame = !self.odd_frame;
                self.frame_count += 1;
                return nmi_triggered;
            }

            if dot > 340 {
                self.dot = 0;
                self.scanline = scanline + 1;
                if self.scanline > 261 {
                    self.scanline = 0;
                    self.odd_frame = !self.odd_frame;
                    self.frame_count += 1;
                }
            }
        }

        nmi_triggered
    }

    #[allow(dead_code)]
    #[inline(always)]
    fn render_pixel(&mut self) {
        let px = self.dot as usize - 1;
        let py = self.scanline as usize;

        // Background pixel
        let (bg_pixel, bg_palette) = if self.mask & 0x08 != 0
            && (px >= 8 || self.mask & 0x02 != 0)
        {
            let bit = 15 - self.x;
            let p0 = ((self.bg_lo_shift >> bit) & 1) as u8;
            let p1 = ((self.bg_hi_shift >> bit) & 1) as u8;
            let a0 = ((self.at_lo_shift >> bit) & 1) as u8;
            let a1 = ((self.at_hi_shift >> bit) & 1) as u8;
            (p0 | (p1 << 1), a0 | (a1 << 1))
        } else {
            (0, 0)
        };

        // Sprite pixel (single packed byte lookup)
        let sp_data = if self.mask & 0x10 != 0
            && (px >= 8 || self.mask & 0x04 != 0)
            && self.sp_line_has_sprites
        {
            unsafe { *self.sp_line.get_unchecked(px) }
        } else {
            0
        };
        let sp_pixel = sp_data & 0x03;

        // Priority mux — compute palette address directly
        let pal_addr = match (bg_pixel != 0, sp_pixel != 0) {
            (false, false) => 0usize,
            (false, true) => {
                let sp_palette = (sp_data >> 2) & 0x07;
                (sp_palette * 4 + sp_pixel) as usize
            }
            (true, false) => (bg_palette * 4 + bg_pixel) as usize,
            (true, true) => {
                // Sprite 0 hit
                if sp_data & 0x40 != 0 && px < 255 {
                    self.status |= 0x40;
                }
                if (sp_data >> 5) & 0x01 == 0 {
                    // Sprite in front
                    let sp_palette = (sp_data >> 2) & 0x07;
                    (sp_palette * 4 + sp_pixel) as usize
                } else {
                    // Background in front
                    (bg_palette * 4 + bg_pixel) as usize
                }
            }
        };

        // Single u32 write from pre-computed palette cache
        let color = unsafe { *self.pal_cache.get_unchecked(pal_addr) };
        let offset = py * SCREEN_W + px;
        // Safety: px < 256, py < 240, offset < 256*240 = framebuffer.len()
        unsafe {
            *self.framebuffer.get_unchecked_mut(offset) = color;
        }
    }

}
