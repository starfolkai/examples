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
    // VRAM
    pub nt_ram: [u8; 2048],     // 2KB nametable RAM
    pub palette: [u8; 32],       // palette RAM
    pub oam: [u8; 256],          // OAM (sprite data)

    // Registers
    pub ctrl: u8,    // $2000 PPUCTRL
    pub mask: u8,    // $2001 PPUMASK
    pub status: u8,  // $2002 PPUSTATUS
    pub oam_addr: u8,

    // Scrolling (loopy registers)
    pub v: u16,      // current VRAM address (15 bits)
    pub t: u16,      // temporary VRAM address
    pub x: u8,       // fine X scroll (3 bits)
    pub w: bool,     // write toggle (first/second write)

    // Internal latches
    pub read_buf: u8,
    pub open_bus: u8,

    // Rendering state
    pub scanline: i32,
    pub dot: u32,
    pub frame_count: u64,
    pub odd_frame: bool,

    // Background shift registers
    bg_lo_shift: u16,
    bg_hi_shift: u16,
    at_lo_shift: u16,
    at_hi_shift: u16,

    // Background tile fetch pipeline
    nt_byte: u8,
    at_byte: u8,
    bg_lo: u8,
    bg_hi: u8,

    // Sprite evaluation
    sprite_count: usize,
    sprite_patterns_lo: [u8; 8],
    sprite_patterns_hi: [u8; 8],
    sprite_positions: [u8; 8],
    sprite_priorities: [u8; 8],
    sprite_indices: [u8; 8],

    // Output
    pub framebuffer: [u8; SCREEN_W * SCREEN_H * 3],

    // NMI
    pub nmi_output: bool,    // from PPUCTRL bit 7
    pub nmi_occurred: bool,  // from PPUSTATUS bit 7
    pub nmi_line: bool,      // actual NMI signal to CPU
    nmi_delay: u8,
}

impl Ppu {
    pub fn new() -> Self {
        Ppu {
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
            sprite_patterns_lo: [0; 8],
            sprite_patterns_hi: [0; 8],
            sprite_positions: [0; 8],
            sprite_priorities: [0; 8],
            sprite_indices: [0; 8],
            framebuffer: [0; SCREEN_W * SCREEN_H * 3],
            nmi_output: false, nmi_occurred: false, nmi_line: false,
            nmi_delay: 0,
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
        for i in 0..256 {
            self.oam[self.oam_addr as usize] = data[i];
            self.oam_addr = self.oam_addr.wrapping_add(1);
        }
    }

    // ── VRAM access ──

    fn vram_read(&self, addr: u16, cart: &Cartridge) -> u8 {
        let a = addr & 0x3FFF;
        if a < 0x2000 {
            cart.read_chr(a)
        } else if a < 0x3F00 {
            let idx = cart.mirror_nt(a & 0x2FFF);
            self.nt_ram[idx]
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
    fn rendering_enabled(&self) -> bool {
        self.mask & 0x18 != 0
    }

    fn fetch_nt_byte(&mut self, cart: &Cartridge) {
        let addr = 0x2000 | (self.v & 0x0FFF);
        let idx = cart.mirror_nt(addr);
        self.nt_byte = self.nt_ram[idx];
    }

    fn fetch_at_byte(&mut self, cart: &Cartridge) {
        let v = self.v;
        let addr = 0x23C0 | (v & 0x0C00) | ((v >> 4) & 0x38) | ((v >> 2) & 7);
        let shift = ((v >> 4) & 4) | (v & 2);
        let idx = cart.mirror_nt(addr);
        self.at_byte = (self.nt_ram[idx] >> shift) & 3;
    }

    fn fetch_tile_lo(&mut self, cart: &Cartridge) {
        let fine_y = (self.v >> 12) & 7;
        let base = if self.ctrl & 0x10 != 0 { 0x1000u16 } else { 0 };
        let addr = base + (self.nt_byte as u16) * 16 + fine_y;
        self.bg_lo = cart.read_chr(addr);
    }

    fn fetch_tile_hi(&mut self, cart: &Cartridge) {
        let fine_y = (self.v >> 12) & 7;
        let base = if self.ctrl & 0x10 != 0 { 0x1000u16 } else { 0 };
        let addr = base + (self.nt_byte as u16) * 16 + fine_y + 8;
        self.bg_hi = cart.read_chr(addr);
    }

    fn load_bg_shifters(&mut self) {
        self.bg_lo_shift = (self.bg_lo_shift & 0xFF00) | self.bg_lo as u16;
        self.bg_hi_shift = (self.bg_hi_shift & 0xFF00) | self.bg_hi as u16;
        // Fill low 8 bits with attribute value (all 1s or all 0s).
        // This ensures correct palette across tile boundaries with fine scroll.
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

    fn increment_x(&mut self) {
        if self.v & 0x001F == 31 {
            self.v &= !0x001F;
            self.v ^= 0x0400; // switch horizontal nametable
        } else {
            self.v += 1;
        }
    }

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

    fn copy_x(&mut self) {
        // v: ....A.. ...BCDEF <- t: ....A.. ...BCDEF
        self.v = (self.v & 0xFBE0) | (self.t & 0x041F);
    }

    fn copy_y(&mut self) {
        // v: GHIA.BC DEF..... <- t: GHIA.BC DEF.....
        self.v = (self.v & 0x841F) | (self.t & 0x7BE0);
    }

    // Sprite evaluation for current scanline
    fn evaluate_sprites(&mut self, cart: &Cartridge) {
        let tall = self.ctrl & 0x20 != 0; // 8x16 sprites
        let h: i32 = if tall { 16 } else { 8 };
        self.sprite_count = 0;

        for i in 0..64 {
            let y = self.oam[i * 4] as i32;
            let row = self.scanline - y;
            if row < 0 || row >= h { continue; }
            if self.sprite_count >= 8 {
                self.status |= 0x20; // sprite overflow
                break;
            }

            let tile_idx = self.oam[i * 4 + 1];
            let attr = self.oam[i * 4 + 2];
            let x = self.oam[i * 4 + 3];

            let flip_h = attr & 0x40 != 0;
            let flip_v = attr & 0x80 != 0;

            let mut row = row as u16;
            if flip_v { row = h as u16 - 1 - row; }

            let (base, tile) = if tall {
                let base = if tile_idx & 1 != 0 { 0x1000u16 } else { 0u16 };
                let tile = tile_idx & 0xFE;
                if row >= 8 {
                    (base, tile + 1)
                } else {
                    (base, tile)
                }
            } else {
                let base = if self.ctrl & 0x08 != 0 { 0x1000u16 } else { 0u16 };
                (base, tile_idx)
            };

            let addr = base + tile as u16 * 16 + (row & 7);
            let mut lo = cart.read_chr(addr);
            let mut hi = cart.read_chr(addr + 8);

            if flip_h {
                lo = lo.reverse_bits();
                hi = hi.reverse_bits();
            }

            let s = self.sprite_count;
            self.sprite_patterns_lo[s] = lo;
            self.sprite_patterns_hi[s] = hi;
            self.sprite_positions[s] = x;
            self.sprite_priorities[s] = (attr >> 5) & 1;
            self.sprite_indices[s] = i as u8;
            self.sprite_count += 1;
        }
    }

    // ── Main tick — call 3x per CPU cycle ──

    pub fn tick(&mut self, cart: &Cartridge) -> bool {
        let mut nmi_triggered = false;

        if self.nmi_delay > 0 {
            self.nmi_delay -= 1;
            if self.nmi_delay == 0 && self.nmi_output && self.nmi_occurred {
                nmi_triggered = true;
            }
        }

        let rendering = self.rendering_enabled();
        let visible_line = self.scanline < 240;
        let pre_line = self.scanline == 261;
        let render_line = visible_line || pre_line;
        let fetch_cycle = (1..=256).contains(&self.dot) || (321..=336).contains(&self.dot);

        if rendering {
            // ── Background rendering ──
            if visible_line && (1..=256).contains(&self.dot) {
                self.render_pixel(cart);
            }

            if render_line && fetch_cycle {
                self.shift_bg();
                match self.dot & 7 {
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

            if render_line {
                if self.dot == 256 {
                    self.increment_y();
                }
                if self.dot == 257 {
                    self.copy_x();
                }
            }

            // Sprite evaluation at dot 257
            if visible_line && self.dot == 257 {
                self.evaluate_sprites(cart);
            }

            // Pre-render: copy Y at dots 280-304
            if pre_line && (280..=304).contains(&self.dot) {
                self.copy_y();
            }
        }

        // ── VBlank flag ──
        if self.scanline == 241 && self.dot == 1 {
            self.status |= 0x80;
            self.nmi_occurred = true;
            self.nmi_change();
        }

        if pre_line && self.dot == 1 {
            self.status &= !0xE0; // clear VBlank, sprite 0, overflow
            self.nmi_occurred = false;
            self.nmi_change();
        }

        // ── Advance dot/scanline ──
        self.dot += 1;

        // Odd frame skip
        if rendering && self.odd_frame && self.scanline == 261 && self.dot == 340 {
            self.dot = 0;
            self.scanline = 0;
            self.odd_frame = !self.odd_frame;
            self.frame_count += 1;
            return nmi_triggered;
        }

        if self.dot > 340 {
            self.dot = 0;
            self.scanline += 1;
            if self.scanline > 261 {
                self.scanline = 0;
                self.odd_frame = !self.odd_frame;
                self.frame_count += 1;
            }
        }

        nmi_triggered
    }

    fn render_pixel(&mut self, _cart: &Cartridge) {
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

        // Sprite pixel
        let (mut sp_pixel, mut sp_palette, mut sp_priority, mut sp_zero) = (0u8, 0u8, 0u8, false);
        if self.mask & 0x10 != 0 && (px >= 8 || self.mask & 0x04 != 0) {
            for s in 0..self.sprite_count {
                let offset = px as i32 - self.sprite_positions[s] as i32;
                if offset < 0 || offset >= 8 { continue; }

                let bit = 7 - offset as u8;
                let p0 = (self.sprite_patterns_lo[s] >> bit) & 1;
                let p1 = (self.sprite_patterns_hi[s] >> bit) & 1;
                let pixel = p0 | (p1 << 1);
                if pixel == 0 { continue; }

                sp_pixel = pixel;
                sp_palette = (self.oam[self.sprite_indices[s] as usize * 4 + 2] & 3) + 4;
                sp_priority = self.sprite_priorities[s];
                sp_zero = self.sprite_indices[s] == 0;
                break;
            }
        }

        // Priority mux
        let (color_idx, palette_idx) = match (bg_pixel != 0, sp_pixel != 0) {
            (false, false) => (0u8, 0u8),
            (false, true) => (sp_pixel, sp_palette),
            (true, false) => (bg_pixel, bg_palette),
            (true, true) => {
                // Sprite 0 hit
                if sp_zero && px < 255 {
                    self.status |= 0x40;
                }
                if sp_priority == 0 {
                    (sp_pixel, sp_palette)
                } else {
                    (bg_pixel, bg_palette)
                }
            }
        };

        let pal_addr = if color_idx == 0 { 0 } else { palette_idx * 4 + color_idx };
        let nes_color = self.palette[pal_addr as usize] & 0x3F;
        let rgb = NES_PAL[nes_color as usize];

        let offset = (py * SCREEN_W + px) * 3;
        if offset + 2 < self.framebuffer.len() {
            self.framebuffer[offset] = rgb[0];
            self.framebuffer[offset + 1] = rgb[1];
            self.framebuffer[offset + 2] = rgb[2];
        }
    }

}
