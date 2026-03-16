// NES system — ties CPU, PPU, bus together
//
// PPU runs at 3x CPU clock. PPU ticks accumulate as debt and are
// processed in scanline-sized batches for maximum throughput.

use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;

pub struct Nes {
    pub cpu: Cpu,
    pub bus: Bus,
    ppu_debt: u32, // accumulated PPU dots not yet processed
}

impl Nes {
    pub fn new(cart: Cartridge) -> Self {
        let mut nes = Nes {
            cpu: Cpu::new(),
            bus: Bus::new(cart),
            ppu_debt: 0,
        };
        nes.cpu.reset(&mut nes.bus);
        nes
    }

    /// Run until one frame completes. Returns frame count.
    #[inline(never)]
    pub fn run_frame(&mut self) -> u64 {
        let target = self.bus.ppu.frame_count + 1;
        while self.bus.ppu.frame_count < target {
            self.step();
        }
        self.bus.ppu.frame_count
    }

    /// Single CPU step + batched PPU scanlines + APU clock
    #[inline(always)]
    fn step(&mut self) {
        // Handle DMA (rare — only after $4014 write)
        if self.bus.dma_pending {
            let cycles = self.bus.do_dma();
            self.cpu.stall += cycles;
        }

        let cpu_cycles = self.cpu.step(&mut self.bus);

        // PPU: accumulate dots and process complete scanlines in batch
        self.ppu_debt += cpu_cycles * 3;
        if self.ppu_debt >= 341 {
            loop {
                self.ppu_debt -= 341;
                let cart = &self.bus.cart;
                if self.bus.ppu.finish_scanline(cart) {
                    self.cpu.nmi_pending = true;
                }
                if self.ppu_debt < 341 { break; }
            }
        }

        // APU: batch clock
        self.bus.apu.clock_batch(cpu_cycles);

        // DMC read + frame IRQ (both rare, combined into one branch)
        if self.bus.apu.dmc_read_pending.is_some() | self.bus.apu.frame_irq {
            if let Some(addr) = self.bus.apu.dmc_read_pending.take() {
                let byte = self.bus.read(addr);
                self.bus.apu.dmc_fill_buffer(byte);
            }
            if self.bus.apu.frame_irq {
                self.cpu.irq_pending = true;
            }
        }
    }

    // Controller: bit layout matches NES standard
    // 0=A, 1=B, 2=Select, 3=Start, 4=Up, 5=Down, 6=Left, 7=Right
    pub fn set_button(&mut self, player: usize, button: u8, pressed: bool) {
        if player > 1 { return; }
        if pressed {
            self.bus.controller[player] |= 1 << button;
        } else {
            self.bus.controller[player] &= !(1 << button);
        }
    }

    pub fn framebuffer(&self) -> &[u32] {
        &self.bus.ppu.framebuffer
    }
}
