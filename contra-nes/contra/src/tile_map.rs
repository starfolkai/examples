// TileMap — native Rust view of NES nametable data
//
// Stores the 2KB of nametable RAM and provides both raw byte access
// (for PPUDATA reads/writes) and semantic tile/palette queries
// (for the renderer). Replaces the raw nt_ram array.

/// Two-screen nametable storage with semantic accessors
/// Each screen: 32×30 tile IDs (960 bytes) + 64 attribute bytes = 1024 bytes
/// Total: 2048 bytes (same as nt_ram)
pub struct TileMap {
    pub data: [u8; 2048],
}

impl TileMap {
    pub fn new() -> Self {
        TileMap { data: [0; 2048] }
    }

    /// Raw byte read (for PPUDATA reads). idx is mirrored offset 0-2047.
    #[inline(always)]
    pub fn read_raw(&self, idx: usize) -> u8 {
        unsafe { *self.data.get_unchecked(idx) }
    }

    /// Raw byte write (for PPUDATA writes). idx is mirrored offset 0-2047.
    #[inline(always)]
    pub fn write_raw(&mut self, idx: usize, val: u8) {
        unsafe { *self.data.get_unchecked_mut(idx) = val; }
    }

    /// Get tile ID at (coarse_x, coarse_y) in the given nametable
    /// nametable: 0-3 (mirrored to 0-1 by caller)
    #[inline(always)]
    pub fn get_tile(&self, nametable: usize, coarse_x: usize, coarse_y: usize) -> u8 {
        let screen_base = (nametable & 1) << 10;  // 0 or 1024
        let offset = coarse_y * 32 + coarse_x;
        unsafe { *self.data.get_unchecked(screen_base + offset) }
    }

    /// Get 2-bit palette index for a tile at (coarse_x, coarse_y)
    #[inline(always)]
    pub fn get_palette(&self, nametable: usize, coarse_x: usize, coarse_y: usize) -> u8 {
        let screen_base = (nametable & 1) << 10;
        let attr_x = coarse_x >> 2;
        let attr_y = coarse_y >> 2;
        let attr_offset = 960 + attr_y * 8 + attr_x;
        let attr = unsafe { *self.data.get_unchecked(screen_base + attr_offset) };
        let shift = ((coarse_y & 2) << 1) | (coarse_x & 2);
        (attr >> shift) & 3
    }
}
