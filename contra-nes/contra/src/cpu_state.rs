// CPU state struct for compiled execution
//
// This replaces the full CPU interpreter. The compiled blocks (generated
// by build.rs) manipulate this state directly. Helper methods handle
// operations that need multi-step logic (ADC, push/pull, etc.)

use crate::bus::Bus;

pub struct CpuState {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub c: bool, pub z: bool, pub i: bool, pub d: bool,
    pub v: bool, pub n: bool,
    pub cycles: u64,
    pub nmi_pending: bool,
    pub irq_pending: bool,
    pub stall: u32,
}

impl CpuState {
    pub fn new() -> Self {
        CpuState {
            a: 0, x: 0, y: 0, sp: 0xFD, pc: 0,
            c: false, z: false, i: true, d: false,
            v: false, n: false,
            cycles: 0, nmi_pending: false, irq_pending: false, stall: 0,
        }
    }

    pub fn reset(&mut self, bus: &mut Bus) {
        let lo = bus.read(0xFFFC) as u16;
        let hi = bus.read(0xFFFD) as u16;
        self.pc = lo | (hi << 8);
        self.sp = 0xFD;
        self.i = true;
        self.cycles = 0;
    }

    #[inline(always)]
    pub fn status(&self) -> u8 {
        let mut s = 0x20u8;
        if self.c { s |= 0x01; }
        if self.z { s |= 0x02; }
        if self.i { s |= 0x04; }
        if self.d { s |= 0x08; }
        if self.v { s |= 0x40; }
        if self.n { s |= 0x80; }
        s
    }

    #[inline(always)]
    pub fn set_status(&mut self, s: u8) {
        self.c = s & 0x01 != 0;
        self.z = s & 0x02 != 0;
        self.i = s & 0x04 != 0;
        self.d = s & 0x08 != 0;
        self.v = s & 0x40 != 0;
        self.n = s & 0x80 != 0;
    }

    #[inline(always)]
    pub fn push(&mut self, bus: &mut Bus, val: u8) {
        unsafe { *bus.ram.get_unchecked_mut(0x100 + self.sp as usize) = val; }
        self.sp = self.sp.wrapping_sub(1);
    }

    #[inline(always)]
    pub fn pull(&mut self, bus: &mut Bus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        unsafe { *bus.ram.get_unchecked(0x100 + self.sp as usize) }
    }

    #[inline(always)]
    pub fn push16(&mut self, bus: &mut Bus, val: u16) {
        self.push(bus, (val >> 8) as u8);
        self.push(bus, val as u8);
    }

    #[inline(always)]
    pub fn pull16(&mut self, bus: &mut Bus) -> u16 {
        let lo = self.pull(bus) as u16;
        let hi = self.pull(bus) as u16;
        lo | (hi << 8)
    }

    #[inline(always)]
    pub fn adc(&mut self, val: u8) {
        let sum = self.a as u16 + val as u16 + self.c as u16;
        self.c = sum > 0xFF;
        self.v = (!(self.a ^ val) & (self.a ^ sum as u8)) & 0x80 != 0;
        self.a = sum as u8;
        self.z = self.a == 0;
        self.n = self.a & 0x80 != 0;
    }

    #[inline(always)]
    pub fn sbc(&mut self, val: u8) {
        self.adc(!val);
    }

    #[inline(always)]
    pub fn cmp_reg(&mut self, reg: u8, val: u8) {
        let diff = reg.wrapping_sub(val);
        self.c = reg >= val;
        self.z = diff == 0;
        self.n = diff & 0x80 != 0;
    }

    pub fn nmi(&mut self, bus: &mut Bus) {
        self.push16(bus, self.pc);
        self.push(bus, self.status() & !0x10);
        self.i = true;
        let lo = bus.read(0xFFFA) as u16;
        let hi = bus.read(0xFFFB) as u16;
        self.pc = lo | (hi << 8);
        self.cycles += 7;
    }

    pub fn irq(&mut self, bus: &mut Bus) {
        self.push16(bus, self.pc);
        self.push(bus, self.status() & !0x10);
        self.i = true;
        let lo = bus.read(0xFFFE) as u16;
        let hi = bus.read(0xFFFF) as u16;
        self.pc = lo | (hi << 8);
        self.cycles += 7;
    }
}
