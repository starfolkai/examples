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

    #[inline(always)]
    pub fn read(&mut self, addr: u16) -> u8 {
        // Fast path: PRG ROM is the most common read (instruction fetches)
        if addr >= 0x8000 {
            return self.cart.read_prg(addr);
        }
        // Second most common: RAM (zero-page, stack, working RAM)
        if addr < 0x2000 {
            return unsafe { *self.ram.get_unchecked((addr & 0x07FF) as usize) };
        }
        // Less common: PPU registers, APU, controllers
        match addr {
            0x2000..=0x3FFF => {
                let cart = &self.cart;
                self.ppu.read_register(addr, cart)
            }
            0x4016 => {
                if self.controller_strobe {
                    self.controller_shift[0] = self.controller[0];
                }
                let val = self.controller_shift[0] & 1;
                self.controller_shift[0] >>= 1;
                self.controller_shift[0] |= 0x80; // open bus pulls high
                val
            }
            0x4017 => {
                if self.controller_strobe {
                    self.controller_shift[1] = self.controller[1];
                }
                let val = self.controller_shift[1] & 1;
                self.controller_shift[1] >>= 1;
                self.controller_shift[1] |= 0x80;
                val
            }
            0x4015 => self.apu.read_status(),
            _ => 0,
        }
    }

    #[inline(always)]
    pub fn write(&mut self, addr: u16, val: u8) {
        // Fast path: RAM writes
        if addr < 0x2000 {
            unsafe { *self.ram.get_unchecked_mut((addr & 0x07FF) as usize) = val; }
            return;
        }
        match addr {
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
                let new_strobe = val & 1 != 0;
                // Latch shift registers on strobe 1→0 transition
                if self.controller_strobe && !new_strobe {
                    self.controller_shift[0] = self.controller[0];
                    self.controller_shift[1] = self.controller[1];
                }
                self.controller_strobe = new_strobe;
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
        if base < 0x2000 {
            // Most common case: DMA from RAM — bypass bus dispatch entirely
            let ram_base = (base & 0x07FF) as usize;
            for i in 0..256usize {
                data[i] = unsafe { *self.ram.get_unchecked((ram_base + i) & 0x07FF) };
            }
        } else {
            for i in 0..256u16 {
                data[i as usize] = self.read(base + i);
            }
        }
        self.ppu.write_oam_dma(&data);
        self.dma_pending = false;
        513 // DMA takes 513-514 CPU cycles
    }
}
