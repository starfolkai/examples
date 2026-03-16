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

        // PPU: 3 ticks per CPU cycle
        // Unroll the common case (most instructions are 2-4 cycles = 6-12 PPU ticks)
        let ppu_ticks = cpu_cycles * 3;
        let cart = &self.bus.cart;
        for _ in 0..ppu_ticks {
            if self.bus.ppu.tick(cart) {
                self.cpu.nmi_pending = true;
            }
        }

        // APU: batch clock (most work is just counter decrements)
        self.bus.apu.clock_batch(cpu_cycles);

        // Handle DMC memory reads (rare — only when DMC channel is playing samples)
        if self.bus.apu.dmc_read_pending.is_some() {
            let addr = self.bus.apu.dmc_read_pending.take().unwrap();
            let byte = self.bus.read(addr);
            self.bus.apu.dmc_fill_buffer(byte);
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
