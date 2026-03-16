// NES system — ties CPU, PPU, bus together
//
// PPU runs at 3x CPU clock. Each CPU step, advance PPU by 3× the cycles consumed.

use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;

pub struct Nes {
    pub cpu: Cpu,
    pub bus: Bus,
}

impl Nes {
    pub fn new(cart: Cartridge) -> Self {
        let mut nes = Nes {
            cpu: Cpu::new(),
            bus: Bus::new(cart),
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

    /// Single CPU step + PPU ticks + APU clock
    #[inline(always)]
    fn step(&mut self) {
        // Handle DMA
        if self.bus.dma_pending {
            let cycles = self.bus.do_dma();
            self.cpu.stall += cycles;
        }

        let cpu_cycles = self.cpu.step(&mut self.bus);
        let ppu_cycles = cpu_cycles * 3;

        for _ in 0..ppu_cycles {
            let cart = &self.bus.cart;
            let nmi = self.bus.ppu.tick(cart);
            if nmi {
                self.cpu.nmi_pending = true;
            }
        }

        // Clock APU once per CPU cycle
        for _ in 0..cpu_cycles {
            self.bus.apu.clock();

            // Handle DMC memory reads
            if let Some(addr) = self.bus.apu.dmc_read_pending.take() {
                let byte = self.bus.read(addr);
                self.bus.apu.dmc_fill_buffer(byte);
            }
        }

        // APU frame IRQ
        if self.bus.apu.frame_irq {
            self.cpu.irq_pending = true;
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

    pub fn framebuffer(&self) -> &[u8] {
        &self.bus.ppu.framebuffer
    }
}
