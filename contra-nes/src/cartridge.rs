// iNES ROM loader + Mapper 2 (UxROM) for Contra
//
// iNES header: 16 bytes, then PRG banks (16KB each), then CHR (if any).
// Contra: Mapper 2, 8x16KB PRG, 0 CHR ROM (uses CHR RAM), vertical mirroring.

pub const PRG_BANK_SIZE: usize = 16384; // 16KB
pub const CHR_RAM_SIZE: usize = 8192;   // 8KB

#[derive(Clone, Copy, PartialEq)]
pub enum Mirroring {
    Horizontal,
    Vertical,
}

pub struct Cartridge {
    pub prg: Vec<u8>,
    pub chr_ram: [u8; CHR_RAM_SIZE],
    pub mirroring: Mirroring,
    pub prg_banks: usize,
    // Mapper 2 state
    pub bank_select: usize, // switchable bank at $8000-$BFFF
}

impl Cartridge {
    pub fn from_ines(data: &[u8]) -> Self {
        assert!(data.len() >= 16, "ROM too small");
        assert!(data[0] == b'N' && data[1] == b'E' && data[2] == b'S' && data[3] == 0x1A,
            "Not iNES format");

        let prg_banks = data[4] as usize;
        let chr_banks = data[5] as usize;
        let flags6 = data[6];
        let flags7 = data[7];

        let mapper = (flags7 & 0xF0) | (flags6 >> 4);
        let mirroring = if flags6 & 1 == 1 { Mirroring::Vertical } else { Mirroring::Horizontal };
        let has_trainer = flags6 & 4 != 0;

        let prg_start = 16 + if has_trainer { 512 } else { 0 };
        let prg_size = prg_banks * PRG_BANK_SIZE;
        let prg = data[prg_start..prg_start + prg_size].to_vec();

        // CHR: Contra has 0 CHR banks (uses CHR RAM)
        let mut chr_ram = [0u8; CHR_RAM_SIZE];
        if chr_banks > 0 {
            let chr_start = prg_start + prg_size;
            let chr_size = chr_banks * 8192;
            let copy_len = chr_size.min(CHR_RAM_SIZE);
            chr_ram[..copy_len].copy_from_slice(&data[chr_start..chr_start + copy_len]);
        }

        eprintln!("  ROM: mapper={}, PRG={}x16KB, CHR_ROM={}x8KB, mirror={:?}",
            mapper, prg_banks, chr_banks,
            if mirroring == Mirroring::Vertical { "V" } else { "H" });

        Cartridge {
            prg,
            chr_ram,
            mirroring,
            prg_banks,
            bank_select: 0,
        }
    }

    // Mapper 2: CPU read from PRG space
    #[inline(always)]
    pub fn read_prg(&self, addr: u16) -> u8 {
        let a = addr as usize;
        if a < 0xC000 {
            // $8000-$BFFF: switchable bank
            self.prg[self.bank_select * PRG_BANK_SIZE + (a - 0x8000)]
        } else {
            // $C000-$FFFF: fixed to last bank
            self.prg[(self.prg_banks - 1) * PRG_BANK_SIZE + (a - 0xC000)]
        }
    }

    // Mapper 2: CPU write selects PRG bank
    #[inline(always)]
    pub fn write_prg(&mut self, _addr: u16, val: u8) {
        self.bank_select = (val as usize) % self.prg_banks;
    }

    // PPU CHR RAM read
    #[inline(always)]
    pub fn read_chr(&self, addr: u16) -> u8 {
        self.chr_ram[addr as usize & 0x1FFF]
    }

    // PPU CHR RAM write
    #[inline(always)]
    pub fn write_chr(&mut self, addr: u16, val: u8) {
        self.chr_ram[addr as usize & 0x1FFF] = val;
    }

    // Nametable mirroring: map PPU $2000-$2FFF to 2KB VRAM
    #[inline(always)]
    pub fn mirror_nt(&self, addr: u16) -> usize {
        let a = (addr - 0x2000) as usize;
        let table = a / 0x400;
        let offset = a % 0x400;
        match self.mirroring {
            Mirroring::Vertical => {
                // Tables 0,2 -> 0; Tables 1,3 -> 1
                ((table & 1) * 0x400) + offset
            }
            Mirroring::Horizontal => {
                // Tables 0,1 -> 0; Tables 2,3 -> 1
                ((table >> 1) * 0x400) + offset
            }
        }
    }
}
