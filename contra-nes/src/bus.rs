// NES system bus — memory map and DMA
//
// $0000-$07FF: 2KB internal RAM (mirrored to $1FFF)
// $2000-$2007: PPU registers (mirrored to $3FFF)
// $4000-$4017: APU + I/O
// $4018-$FFFF: Cartridge space (mapper handles banking)

use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::ppu::Ppu;

pub struct Bus {
    pub ram: [u8; 2048],
    pub cart: Cartridge,
    pub ppu: Ppu,
    pub apu: Apu,
    pub controller: [u8; 2],     // current button state
    pub controller_shift: [u8; 2], // shift register for reads
    pub controller_strobe: bool,
    pub dma_page: u8,
    pub dma_pending: bool,
}

impl Bus {
    pub fn new(cart: Cartridge) -> Self {
        Bus {
            ram: [0; 2048],
            ppu: Ppu::new(),
            apu: Apu::new(),
            cart,
            controller: [0; 2],
            controller_shift: [0; 2],
            controller_strobe: false,
            dma_page: 0,
            dma_pending: false,
        }
    }

    pub fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize],
            0x2000..=0x3FFF => {
                let cart = &self.cart;
                self.ppu.read_register(addr, cart)
            }
            0x4016 => {
                let val = self.controller_shift[0] & 1;
                self.controller_shift[0] >>= 1;
                self.controller_shift[0] |= 0x80; // open bus pulls high
                val
            }
            0x4017 => {
                let val = self.controller_shift[1] & 1;
                self.controller_shift[1] >>= 1;
                self.controller_shift[1] |= 0x80;
                val
            }
            0x4015 => self.apu.read_status(),
            0x4000..=0x4014 => 0, // APU write-only registers
            0x4018..=0x5FFF => 0, // expansion (unused by Contra)
            0x6000..=0x7FFF => 0, // PRG RAM (Contra doesn't use it)
            0x8000..=0xFFFF => self.cart.read_prg(addr),
        }
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram[(addr & 0x07FF) as usize] = val,
            0x2000..=0x3FFF => {
                let cart = &mut self.cart;
                self.ppu.write_register(addr, val, cart);
            }
            0x4014 => {
                // OAM DMA — copy 256 bytes from CPU page to OAM
                self.dma_page = val;
                self.dma_pending = true;
            }
            0x4016 => {
                self.controller_strobe = val & 1 != 0;
                if self.controller_strobe {
                    self.controller_shift[0] = self.controller[0];
                    self.controller_shift[1] = self.controller[1];
                }
            }
            0x4000..=0x4013 | 0x4015 | 0x4017 => {
                self.apu.write_register(addr, val);
            }
            0x8000..=0xFFFF => self.cart.write_prg(addr, val),
            _ => {}
        }
    }

    pub fn do_dma(&mut self) -> u32 {
        let base = (self.dma_page as u16) << 8;
        let mut data = [0u8; 256];
        for i in 0..256u16 {
            data[i as usize] = self.read(base + i);
        }
        self.ppu.write_oam_dma(&data);
        self.dma_pending = false;
        513 // DMA takes 513-514 CPU cycles
    }
}
