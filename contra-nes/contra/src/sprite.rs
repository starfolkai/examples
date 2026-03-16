// Sprite — native Rust representation of NES OAM sprites
//
// Instead of raw OAM byte array, sprites are parsed into typed structs.
// The renderer uses these for drawing.

/// A single sprite parsed from OAM
#[derive(Clone, Copy, Default)]
pub struct Sprite {
    pub y: u8,
    pub tile: u8,
    pub palette: u8,      // 0-3 (maps to palette 4-7)
    pub behind_bg: bool,
    pub flip_h: bool,
    pub flip_v: bool,
    pub x: u8,
}

/// Parsed sprite list from OAM data
pub struct SpriteList {
    pub sprites: [Sprite; 64],
}

impl SpriteList {
    pub fn new() -> Self {
        SpriteList {
            sprites: [Sprite::default(); 64],
        }
    }

    /// Parse all 64 sprites from raw OAM bytes
    #[inline]
    pub fn parse_oam(&mut self, oam: &[u8; 256]) {
        for i in 0..64 {
            let base = i * 4;
            let attr = oam[base + 2];
            self.sprites[i] = Sprite {
                y: oam[base],
                tile: oam[base + 1],
                x: oam[base + 3],
                palette: attr & 3,
                behind_bg: attr & 0x20 != 0,
                flip_h: attr & 0x40 != 0,
                flip_v: attr & 0x80 != 0,
            };
        }
    }
}
