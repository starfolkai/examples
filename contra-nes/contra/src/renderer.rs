// Frame renderer — draws tiles and sprites using native Rust structs
//
// Replaces the PPU's shift-register scanline rendering with direct
// tile/sprite drawing. Still called per-scanline to maintain timing
// compatibility (sprite-zero hit, scroll splits).

use crate::cartridge::Cartridge;
use crate::sprite::SpriteList;
use crate::tile_map::TileMap;

const SCREEN_W: usize = 256;

// NES master palette
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

pub struct Renderer {
    pub pal_cache: [u32; 32],
    sp_line: [u8; SCREEN_W],
    sp_line_has_sprites: bool,
}

impl Renderer {
    pub fn new() -> Self {
        Renderer {
            pal_cache: [0; 32],
            sp_line: [0; SCREEN_W],
            sp_line_has_sprites: false,
        }
    }

    pub fn rebuild_pal_cache(&mut self, palette: &[u8; 32]) {
        for i in 0..32 {
            let c = (palette[i] & 0x3F) as usize;
            let rgb = unsafe { NES_PAL.get_unchecked(c) };
            self.pal_cache[i] = (rgb[0] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[2] as u32;
        }
    }

    /// Evaluate sprites for a scanline — fill sp_line buffer
    pub fn evaluate_sprites(
        &mut self,
        sprites: &SpriteList,
        scanline: i32,
        tall_sprites: bool,
        sprite_base: u16,
        ctrl: u8,
        cart: &Cartridge,
    ) {
        if self.sp_line_has_sprites {
            self.sp_line = [0; SCREEN_W];
        }
        self.sp_line_has_sprites = false;

        let h: i32 = if tall_sprites { 16 } else { 8 };
        let mut count = 0;

        for i in 0..64usize {
            let sp = &sprites.sprites[i];
            let row = scanline - sp.y as i32;
            if row < 0 || row >= h { continue; }
            if count >= 8 { break; }

            let mut row = row as u16;
            if sp.flip_v { row = h as u16 - 1 - row; }

            let (base, tile) = if tall_sprites {
                let base = if sp.tile & 1 != 0 { 0x1000u16 } else { 0u16 };
                let tile = sp.tile & 0xFE;
                if row >= 8 { (base, tile + 1) } else { (base, tile) }
            } else {
                (sprite_base, sp.tile)
            };

            let addr = base + tile as u16 * 16 + (row & 7);
            let mut lo = cart.read_chr(addr);
            let mut hi = cart.read_chr(addr + 8);

            if lo | hi == 0 { count += 1; continue; }
            if sp.flip_h { lo = lo.reverse_bits(); hi = hi.reverse_bits(); }

            let pal_bits = (sp.palette + 4) << 2;
            let prio_bits: u8 = if sp.behind_bg { 0x20 } else { 0 };
            let zero_flag: u8 = if i == 0 { 0x40 } else { 0 };
            let combined = pal_bits | prio_bits | zero_flag;

            for dx in 0..8u8 {
                let px = sp.x as usize + dx as usize;
                if px >= SCREEN_W { break; }
                let existing = unsafe { *self.sp_line.get_unchecked(px) };
                if existing & 0x03 != 0 { continue; }

                let bit = 7 - dx;
                let pixel = ((lo >> bit) & 1) | (((hi >> bit) & 1) << 1);
                if pixel == 0 { continue; }

                unsafe { *self.sp_line.get_unchecked_mut(px) = pixel | combined; }
            }
            count += 1;
        }

        self.sp_line_has_sprites = count > 0;
    }

    /// Render one scanline. Returns sprite-zero hit flag.
    #[inline(always)]
    pub fn render_scanline(
        &mut self,
        tile_map: &TileMap,
        cart: &Cartridge,
        framebuffer: &mut [u32],
        scanline: usize,
        v: u16,
        fine_x: u8,
        ctrl: u8,
        mask: u8,
    ) -> bool {
        let show_bg = mask & 0x08 != 0;
        let show_sp = mask & 0x10 != 0;
        let has_sprites = self.sp_line_has_sprites && show_sp;

        if !has_sprites {
            self.render_scanline_bg_only(tile_map, cart, framebuffer, scanline, v, fine_x, ctrl, mask);
            false
        } else {
            self.render_scanline_full(tile_map, cart, framebuffer, scanline, v, fine_x, ctrl, mask)
        }
    }

    // Background-only path — no sprite checks
    #[inline(always)]
    fn render_scanline_bg_only(
        &self,
        tile_map: &TileMap,
        cart: &Cartridge,
        framebuffer: &mut [u32],
        scanline: usize,
        v: u16,
        fine_x: u8,
        ctrl: u8,
        mask: u8,
    ) {
        let show_bg = mask & 0x08 != 0;
        let show_bg_left = mask & 0x02 != 0;
        let bg_base = if ctrl & 0x10 != 0 { 0x1000u16 } else { 0 };
        let bg_color = unsafe { *self.pal_cache.get_unchecked(0) };
        let fb_row = scanline * SCREEN_W;

        let mut coarse_x = (v & 0x1F) as usize;
        let nametable_bit = ((v >> 10) & 1) as usize;
        let mut nametable = nametable_bit;
        let coarse_y = ((v >> 5) & 0x1F) as usize;
        let fine_y = ((v >> 12) & 7) as u16;

        let mut pixel_out = 0usize;
        let mut skip = fine_x as usize;

        while pixel_out < 256 {
            let tile_id = tile_map.get_tile(nametable, coarse_x, coarse_y);
            let chr_addr = bg_base + (tile_id as u16) * 16 + fine_y;
            let lo = cart.read_chr(chr_addr);
            let hi = cart.read_chr(chr_addr + 8);

            let end = (pixel_out + 8 - skip).min(256);

            if show_bg && (lo | hi != 0) {
                let palette_idx = tile_map.get_palette(nametable, coarse_x, coarse_y);
                let pal_base = palette_idx as usize * 4;

                for sub in skip..8 {
                    if pixel_out >= 256 { break; }
                    let bit = 7 - sub as u8;
                    let p = ((lo >> bit) & 1) | (((hi >> bit) & 1) << 1);
                    let color = if p != 0 && (pixel_out >= 8 || show_bg_left) {
                        unsafe { *self.pal_cache.get_unchecked(pal_base + p as usize) }
                    } else {
                        bg_color
                    };
                    unsafe { *framebuffer.get_unchecked_mut(fb_row + pixel_out) = color; }
                    pixel_out += 1;
                }
            } else {
                // Entirely transparent tile — fill with bg color
                for _ in skip..8 {
                    if pixel_out >= 256 { break; }
                    unsafe { *framebuffer.get_unchecked_mut(fb_row + pixel_out) = bg_color; }
                    pixel_out += 1;
                }
            }

            skip = 0;
            coarse_x += 1;
            if coarse_x >= 32 {
                coarse_x = 0;
                nametable ^= 1;
            }
        }
    }

    // Full path with sprite priority mux
    #[inline(always)]
    fn render_scanline_full(
        &mut self,
        tile_map: &TileMap,
        cart: &Cartridge,
        framebuffer: &mut [u32],
        scanline: usize,
        v: u16,
        fine_x: u8,
        ctrl: u8,
        mask: u8,
    ) -> bool {
        let show_bg = mask & 0x08 != 0;
        let show_sp = mask & 0x10 != 0;
        let show_bg_left = mask & 0x02 != 0;
        let show_sp_left = mask & 0x04 != 0;
        let bg_base = if ctrl & 0x10 != 0 { 0x1000u16 } else { 0 };
        let bg_color = unsafe { *self.pal_cache.get_unchecked(0) };
        let fb_row = scanline * SCREEN_W;

        let mut coarse_x = (v & 0x1F) as usize;
        let nametable_bit = ((v >> 10) & 1) as usize;
        let mut nametable = nametable_bit;
        let coarse_y = ((v >> 5) & 0x1F) as usize;
        let fine_y = ((v >> 12) & 7) as u16;

        let mut pixel_out = 0usize;
        let mut skip = fine_x as usize;
        let mut sprite_zero_hit = false;

        while pixel_out < 256 {
            let tile_id = tile_map.get_tile(nametable, coarse_x, coarse_y);
            let palette_idx = tile_map.get_palette(nametable, coarse_x, coarse_y);
            let chr_addr = bg_base + (tile_id as u16) * 16 + fine_y;
            let lo = cart.read_chr(chr_addr);
            let hi = cart.read_chr(chr_addr + 8);

            for sub in skip..8 {
                if pixel_out >= 256 { break; }
                let px = pixel_out;

                let bit = 7 - sub as u8;
                let bg_pixel = if show_bg && (px >= 8 || show_bg_left) {
                    ((lo >> bit) & 1) | (((hi >> bit) & 1) << 1)
                } else {
                    0
                };

                let sp_data = if show_sp && (px >= 8 || show_sp_left) {
                    unsafe { *self.sp_line.get_unchecked(px) }
                } else {
                    0
                };
                let sp_pixel = sp_data & 0x03;

                let color = match (bg_pixel != 0, sp_pixel != 0) {
                    (false, false) => bg_color,
                    (false, true) => {
                        let pal = ((sp_data >> 2) & 7) as usize * 4 + sp_pixel as usize;
                        unsafe { *self.pal_cache.get_unchecked(pal) }
                    }
                    (true, false) => {
                        unsafe { *self.pal_cache.get_unchecked(palette_idx as usize * 4 + bg_pixel as usize) }
                    }
                    (true, true) => {
                        if sp_data & 0x40 != 0 && px < 255 {
                            sprite_zero_hit = true;
                        }
                        if (sp_data >> 5) & 1 == 0 {
                            let pal = ((sp_data >> 2) & 7) as usize * 4 + sp_pixel as usize;
                            unsafe { *self.pal_cache.get_unchecked(pal) }
                        } else {
                            unsafe { *self.pal_cache.get_unchecked(palette_idx as usize * 4 + bg_pixel as usize) }
                        }
                    }
                };

                unsafe { *framebuffer.get_unchecked_mut(fb_row + px) = color; }
                pixel_out += 1;
            }

            skip = 0;
            coarse_x += 1;
            if coarse_x >= 32 {
                coarse_x = 0;
                nametable ^= 1;
            }
        }

        sprite_zero_hit
    }
}
