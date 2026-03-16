// NES PPU (2C02) — native Rust rendering
//
// Register interface and timing are hardware-accurate.
// Rendering uses native Rust abstractions (TileMap, SpriteList, Renderer)
// instead of shift-register scanline emulation.

use crate::cartridge::Cartridge;
use crate::renderer::Renderer;
use crate::sprite::SpriteList;
use crate::tile_map::TileMap;

const SCREEN_W: usize = 256;
const SCREEN_H: usize = 240;

pub struct Ppu {
    // Native Rust data structures (replace raw nt_ram + rendering internals)
    pub tile_map: TileMap,
    pub sprites: SpriteList,
    pub renderer: Renderer,

    // PPU state (register interface + timing)
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
    pub framebuffer: [u32; SCREEN_W * SCREEN_H],
    pub nmi_output: bool,
    pub nmi_occurred: bool,
    pub nmi_line: bool,
    nmi_delay: u8,
}

impl Ppu {
    pub fn new() -> Self {
        let mut ppu = Ppu {
            tile_map: TileMap::new(),
            sprites: SpriteList::new(),
            renderer: Renderer::new(),
            palette: [0; 32],
            oam: [0; 256],
            ctrl: 0, mask: 0, status: 0, oam_addr: 0,
            v: 0, t: 0, x: 0, w: false,
            read_buf: 0, open_bus: 0,
            scanline: 0, dot: 0, frame_count: 0, odd_frame: false,
            framebuffer: [0; SCREEN_W * SCREEN_H],
            nmi_output: false, nmi_occurred: false, nmi_line: false,
            nmi_delay: 0,
        };
        ppu.renderer.rebuild_pal_cache(&ppu.palette);
        ppu
    }

    // ── Register reads ──

    pub fn read_register(&mut self, addr: u16, cart: &Cartridge) -> u8 {
        match addr & 7 {
            2 => {
                let mut val = self.status & 0xE0;
                val |= self.open_bus & 0x1F;
                self.nmi_occurred = false;
                self.status &= !0x80;
                self.nmi_change();
                self.w = false;
                self.open_bus = val;
                val
            }
            4 => {
                let val = self.oam[self.oam_addr as usize];
                self.open_bus = val;
                val
            }
            7 => {
                let mut val = self.vram_read(self.v, cart);
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
            0 => {
                self.ctrl = val;
                self.nmi_output = val & 0x80 != 0;
                self.nmi_change();
                self.t = (self.t & 0xF3FF) | ((val as u16 & 3) << 10);
            }
            1 => { self.mask = val; }
            3 => { self.oam_addr = val; }
            4 => {
                self.oam[self.oam_addr as usize] = val;
                self.oam_addr = self.oam_addr.wrapping_add(1);
            }
            5 => {
                if !self.w {
                    self.t = (self.t & 0xFFE0) | ((val as u16) >> 3);
                    self.x = val & 7;
                } else {
                    self.t = (self.t & 0x8C1F) | ((val as u16 & 7) << 12)
                        | ((val as u16 >> 3) << 5);
                }
                self.w = !self.w;
            }
            6 => {
                if !self.w {
                    self.t = (self.t & 0x00FF) | ((val as u16 & 0x3F) << 8);
                } else {
                    self.t = (self.t & 0xFF00) | val as u16;
                    self.v = self.t;
                }
                self.w = !self.w;
            }
            7 => {
                self.vram_write(self.v, val, cart);
                self.v = self.v.wrapping_add(if self.ctrl & 4 != 0 { 32 } else { 1 });
            }
            _ => {}
        }
    }

    pub fn write_oam_dma(&mut self, data: &[u8; 256]) {
        if self.oam_addr == 0 {
            self.oam.copy_from_slice(data);
        } else {
            let start = self.oam_addr as usize;
            let first = 256 - start;
            self.oam[start..].copy_from_slice(&data[..first]);
            self.oam[..start].copy_from_slice(&data[first..]);
        }
    }

    // ── VRAM access (uses TileMap for nametable storage) ──

    fn vram_read(&self, addr: u16, cart: &Cartridge) -> u8 {
        let a = addr & 0x3FFF;
        if a < 0x2000 {
            cart.read_chr(a)
        } else if a < 0x3F00 {
            let idx = cart.mirror_nt(a & 0x2FFF);
            self.tile_map.read_raw(idx)
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
            self.tile_map.write_raw(idx, val);
        } else {
            let mut pa = (a - 0x3F00) as usize & 0x1F;
            if pa == 0x10 || pa == 0x14 || pa == 0x18 || pa == 0x1C { pa &= 0x0F; }
            self.palette[pa] = val;
            self.renderer.rebuild_pal_cache(&self.palette);
        }
    }

    fn nmi_change(&mut self) {
        let nmi = self.nmi_output && self.nmi_occurred;
        if nmi && !self.nmi_line {
            self.nmi_delay = 15;
        }
        self.nmi_line = nmi;
    }

    // ── Scroll register helpers (used for scanline rendering) ──

    #[inline(always)]
    fn increment_x(&mut self) {
        if self.v & 0x001F == 31 {
            self.v &= !0x001F;
            self.v ^= 0x0400;
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

    // ── Scanline-batched processing ──

    pub fn finish_scanline(&mut self, cart: &Cartridge) -> bool {
        let mut nmi = false;
        let sl = self.scanline;
        let rendering = self.mask & 0x18 != 0;

        if self.nmi_delay > 0 {
            self.nmi_delay = 0;
            if self.nmi_output && self.nmi_occurred {
                nmi = true;
            }
        }

        if sl < 240 {
            // ── Visible scanline ──
            if rendering {
                // Render background + compose with pre-evaluated sprites
                let hit = self.renderer.render_scanline(
                    &self.tile_map,
                    cart,
                    &mut self.framebuffer,
                    sl as usize,
                    self.v,
                    self.x,
                    self.ctrl,
                    self.mask,
                );
                if hit { self.status |= 0x40; }

                // Advance scroll for next scanline
                self.increment_y();
                self.copy_x();

                // Evaluate sprites for next scanline (uses current scanline
                // number; NES Y-1 convention means these are correct for N+1)
                self.sprites.parse_oam(&self.oam);
                let tall = self.ctrl & 0x20 != 0;
                let sp_base = if self.ctrl & 0x08 != 0 { 0x1000u16 } else { 0 };
                self.renderer.evaluate_sprites(
                    &self.sprites, sl, tall, sp_base, self.ctrl, cart,
                );
            }
        } else if sl == 241 {
            // ── VBlank start ──
            self.status |= 0x80;
            self.nmi_occurred = true;
            self.nmi_change();
        } else if sl == 261 {
            // ── Pre-render scanline ──
            self.status &= !0xE0;
            self.nmi_occurred = false;
            self.nmi_change();

            if rendering {
                self.copy_y();
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
}
