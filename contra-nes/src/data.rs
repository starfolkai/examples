// Contra PRG ROM data — 8 x 16KB banks, extracted from the original ROM.
// This is compiled into the binary so no .nes file is needed at runtime.
pub const PRG_DATA: &[u8] = include_bytes!("contra_prg.bin");
pub const PRG_BANKS: usize = 8;

// Tile data is embedded in PRG banks 5 and 6
pub const TILE_BANK_0: usize = 5;
pub const TILE_BANK_1: usize = 6;
