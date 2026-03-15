// MOS 6502 CPU — cycle-stepped, no illegal opcodes
//
// Compact decode via match. All addressing modes inline.
// Contra uses standard opcodes only. No BCD (NES lacks it).

use crate::bus::Bus;

pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    // Flags: NV_BDIZC
    pub c: bool, pub z: bool, pub i: bool, pub d: bool,
    pub v: bool, pub n: bool,
    pub cycles: u64,
    pub nmi_pending: bool,
    pub irq_pending: bool,
    pub stall: u32, // DMA stall cycles
}

impl Cpu {
    pub fn new() -> Self {
        Cpu {
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
    fn status(&self) -> u8 {
        let mut s = 0x20u8; // bit 5 always set
        if self.c { s |= 0x01; }
        if self.z { s |= 0x02; }
        if self.i { s |= 0x04; }
        if self.d { s |= 0x08; }
        if self.v { s |= 0x40; }
        if self.n { s |= 0x80; }
        s
    }

    #[inline(always)]
    fn set_status(&mut self, s: u8) {
        self.c = s & 0x01 != 0;
        self.z = s & 0x02 != 0;
        self.i = s & 0x04 != 0;
        self.d = s & 0x08 != 0;
        self.v = s & 0x40 != 0;
        self.n = s & 0x80 != 0;
    }

    #[inline(always)]
    fn push(&mut self, bus: &mut Bus, val: u8) {
        bus.write(0x100 + self.sp as u16, val);
        self.sp = self.sp.wrapping_sub(1);
    }

    #[inline(always)]
    fn pull(&mut self, bus: &mut Bus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        bus.read(0x100 + self.sp as u16)
    }

    #[inline(always)]
    fn push16(&mut self, bus: &mut Bus, val: u16) {
        self.push(bus, (val >> 8) as u8);
        self.push(bus, val as u8);
    }

    #[inline(always)]
    fn pull16(&mut self, bus: &mut Bus) -> u16 {
        let lo = self.pull(bus) as u16;
        let hi = self.pull(bus) as u16;
        lo | (hi << 8)
    }

    #[inline(always)]
    fn set_zn(&mut self, val: u8) {
        self.z = val == 0;
        self.n = val & 0x80 != 0;
    }

    #[inline(always)]
    fn read16(&self, bus: &mut Bus, addr: u16) -> u16 {
        let lo = bus.read(addr) as u16;
        let hi = bus.read(addr.wrapping_add(1)) as u16;
        lo | (hi << 8)
    }

    // Bug-compatible 6502 indirect read (wraps within page)
    #[inline(always)]
    fn read16_bug(&self, bus: &mut Bus, addr: u16) -> u16 {
        let lo = bus.read(addr) as u16;
        let hi_addr = (addr & 0xFF00) | ((addr + 1) & 0x00FF);
        let hi = bus.read(hi_addr) as u16;
        lo | (hi << 8)
    }

    fn nmi(&mut self, bus: &mut Bus) {
        self.push16(bus, self.pc);
        self.push(bus, self.status() & !0x10); // clear B flag
        self.i = true;
        self.pc = self.read16(bus, 0xFFFA);
        self.cycles += 7;
    }

    fn irq(&mut self, bus: &mut Bus) {
        self.push16(bus, self.pc);
        self.push(bus, self.status() & !0x10);
        self.i = true;
        self.pc = self.read16(bus, 0xFFFE);
        self.cycles += 7;
    }

    // Returns cycles consumed by this step
    pub fn step(&mut self, bus: &mut Bus) -> u32 {
        if self.stall > 0 {
            self.stall -= 1;
            self.cycles += 1;
            return 1;
        }

        if self.nmi_pending {
            self.nmi_pending = false;
            self.nmi(bus);
        } else if self.irq_pending && !self.i {
            self.irq(bus);
        }

        let start = self.cycles;
        let op = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);

        // Addressing mode helpers (inline in each opcode for perf, but factor out common patterns)
        macro_rules! imm {
            () => {{ let v = self.pc; self.pc = self.pc.wrapping_add(1); v }}
        }
        macro_rules! zp {
            () => {{ let v = bus.read(self.pc) as u16; self.pc = self.pc.wrapping_add(1); v }}
        }
        macro_rules! zpx {
            () => {{ let v = bus.read(self.pc).wrapping_add(self.x) as u16; self.pc = self.pc.wrapping_add(1); v }}
        }
        macro_rules! zpy {
            () => {{ let v = bus.read(self.pc).wrapping_add(self.y) as u16; self.pc = self.pc.wrapping_add(1); v }}
        }
        macro_rules! abs {
            () => {{ let v = self.read16(bus, self.pc); self.pc = self.pc.wrapping_add(2); v }}
        }
        macro_rules! abx {
            () => {{
                let base = self.read16(bus, self.pc);
                self.pc = self.pc.wrapping_add(2);
                let addr = base.wrapping_add(self.x as u16);
                if base & 0xFF00 != addr & 0xFF00 { self.cycles += 1; }
                addr
            }}
        }
        macro_rules! abx_nopage {
            () => {{
                let base = self.read16(bus, self.pc);
                self.pc = self.pc.wrapping_add(2);
                base.wrapping_add(self.x as u16)
            }}
        }
        macro_rules! aby {
            () => {{
                let base = self.read16(bus, self.pc);
                self.pc = self.pc.wrapping_add(2);
                let addr = base.wrapping_add(self.y as u16);
                if base & 0xFF00 != addr & 0xFF00 { self.cycles += 1; }
                addr
            }}
        }
        macro_rules! aby_nopage {
            () => {{
                let base = self.read16(bus, self.pc);
                self.pc = self.pc.wrapping_add(2);
                base.wrapping_add(self.y as u16)
            }}
        }
        macro_rules! izx {
            () => {{
                let ptr = bus.read(self.pc).wrapping_add(self.x);
                self.pc = self.pc.wrapping_add(1);
                let lo = bus.read(ptr as u16) as u16;
                let hi = bus.read(ptr.wrapping_add(1) as u16) as u16;
                lo | (hi << 8)
            }}
        }
        macro_rules! izy {
            () => {{
                let ptr = bus.read(self.pc);
                self.pc = self.pc.wrapping_add(1);
                let lo = bus.read(ptr as u16) as u16;
                let hi = bus.read(ptr.wrapping_add(1) as u16) as u16;
                let base = lo | (hi << 8);
                let addr = base.wrapping_add(self.y as u16);
                if base & 0xFF00 != addr & 0xFF00 { self.cycles += 1; }
                addr
            }}
        }
        macro_rules! izy_nopage {
            () => {{
                let ptr = bus.read(self.pc);
                self.pc = self.pc.wrapping_add(1);
                let lo = bus.read(ptr as u16) as u16;
                let hi = bus.read(ptr.wrapping_add(1) as u16) as u16;
                let base = lo | (hi << 8);
                base.wrapping_add(self.y as u16)
            }}
        }
        macro_rules! branch {
            ($cond:expr) => {{
                let offset = bus.read(self.pc) as i8;
                self.pc = self.pc.wrapping_add(1);
                if $cond {
                    let new_pc = self.pc.wrapping_add(offset as u16);
                    self.cycles += 1;
                    if self.pc & 0xFF00 != new_pc & 0xFF00 { self.cycles += 1; }
                    self.pc = new_pc;
                }
                self.cycles += 2;
            }}
        }

        match op {
            // ── Load/Store ──
            0xA9 => { let a = imm!(); self.a = bus.read(a); self.set_zn(self.a); self.cycles += 2; }
            0xA5 => { let a = zp!(); self.a = bus.read(a); self.set_zn(self.a); self.cycles += 3; }
            0xB5 => { let a = zpx!(); self.a = bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0xAD => { let a = abs!(); self.a = bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0xBD => { let a = abx!(); self.a = bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0xB9 => { let a = aby!(); self.a = bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0xA1 => { let a = izx!(); self.a = bus.read(a); self.set_zn(self.a); self.cycles += 6; }
            0xB1 => { let a = izy!(); self.a = bus.read(a); self.set_zn(self.a); self.cycles += 5; }

            0xA2 => { let a = imm!(); self.x = bus.read(a); self.set_zn(self.x); self.cycles += 2; }
            0xA6 => { let a = zp!(); self.x = bus.read(a); self.set_zn(self.x); self.cycles += 3; }
            0xB6 => { let a = zpy!(); self.x = bus.read(a); self.set_zn(self.x); self.cycles += 4; }
            0xAE => { let a = abs!(); self.x = bus.read(a); self.set_zn(self.x); self.cycles += 4; }
            0xBE => { let a = aby!(); self.x = bus.read(a); self.set_zn(self.x); self.cycles += 4; }

            0xA0 => { let a = imm!(); self.y = bus.read(a); self.set_zn(self.y); self.cycles += 2; }
            0xA4 => { let a = zp!(); self.y = bus.read(a); self.set_zn(self.y); self.cycles += 3; }
            0xB4 => { let a = zpx!(); self.y = bus.read(a); self.set_zn(self.y); self.cycles += 4; }
            0xAC => { let a = abs!(); self.y = bus.read(a); self.set_zn(self.y); self.cycles += 4; }
            0xBC => { let a = abx!(); self.y = bus.read(a); self.set_zn(self.y); self.cycles += 4; }

            0x85 => { let a = zp!(); bus.write(a, self.a); self.cycles += 3; }
            0x95 => { let a = zpx!(); bus.write(a, self.a); self.cycles += 4; }
            0x8D => { let a = abs!(); bus.write(a, self.a); self.cycles += 4; }
            0x9D => { let a = abx_nopage!(); bus.write(a, self.a); self.cycles += 5; }
            0x99 => { let a = aby_nopage!(); bus.write(a, self.a); self.cycles += 5; }
            0x81 => { let a = izx!(); bus.write(a, self.a); self.cycles += 6; }
            0x91 => { let a = izy_nopage!(); bus.write(a, self.a); self.cycles += 6; }

            0x86 => { let a = zp!(); bus.write(a, self.x); self.cycles += 3; }
            0x96 => { let a = zpy!(); bus.write(a, self.x); self.cycles += 4; }
            0x8E => { let a = abs!(); bus.write(a, self.x); self.cycles += 4; }

            0x84 => { let a = zp!(); bus.write(a, self.y); self.cycles += 3; }
            0x94 => { let a = zpx!(); bus.write(a, self.y); self.cycles += 4; }
            0x8C => { let a = abs!(); bus.write(a, self.y); self.cycles += 4; }

            // ── Arithmetic ──
            0x69 => { let a = imm!(); let v = bus.read(a); self.adc(v); self.cycles += 2; }
            0x65 => { let a = zp!(); let v = bus.read(a); self.adc(v); self.cycles += 3; }
            0x75 => { let a = zpx!(); let v = bus.read(a); self.adc(v); self.cycles += 4; }
            0x6D => { let a = abs!(); let v = bus.read(a); self.adc(v); self.cycles += 4; }
            0x7D => { let a = abx!(); let v = bus.read(a); self.adc(v); self.cycles += 4; }
            0x79 => { let a = aby!(); let v = bus.read(a); self.adc(v); self.cycles += 4; }
            0x61 => { let a = izx!(); let v = bus.read(a); self.adc(v); self.cycles += 6; }
            0x71 => { let a = izy!(); let v = bus.read(a); self.adc(v); self.cycles += 5; }

            0xE9 => { let a = imm!(); let v = bus.read(a); self.sbc(v); self.cycles += 2; }
            0xE5 => { let a = zp!(); let v = bus.read(a); self.sbc(v); self.cycles += 3; }
            0xF5 => { let a = zpx!(); let v = bus.read(a); self.sbc(v); self.cycles += 4; }
            0xED => { let a = abs!(); let v = bus.read(a); self.sbc(v); self.cycles += 4; }
            0xFD => { let a = abx!(); let v = bus.read(a); self.sbc(v); self.cycles += 4; }
            0xF9 => { let a = aby!(); let v = bus.read(a); self.sbc(v); self.cycles += 4; }
            0xE1 => { let a = izx!(); let v = bus.read(a); self.sbc(v); self.cycles += 6; }
            0xF1 => { let a = izy!(); let v = bus.read(a); self.sbc(v); self.cycles += 5; }

            // ── Compare ──
            0xC9 => { let a = imm!(); let v = bus.read(a); self.cmp(self.a, v); self.cycles += 2; }
            0xC5 => { let a = zp!(); let v = bus.read(a); self.cmp(self.a, v); self.cycles += 3; }
            0xD5 => { let a = zpx!(); let v = bus.read(a); self.cmp(self.a, v); self.cycles += 4; }
            0xCD => { let a = abs!(); let v = bus.read(a); self.cmp(self.a, v); self.cycles += 4; }
            0xDD => { let a = abx!(); let v = bus.read(a); self.cmp(self.a, v); self.cycles += 4; }
            0xD9 => { let a = aby!(); let v = bus.read(a); self.cmp(self.a, v); self.cycles += 4; }
            0xC1 => { let a = izx!(); let v = bus.read(a); self.cmp(self.a, v); self.cycles += 6; }
            0xD1 => { let a = izy!(); let v = bus.read(a); self.cmp(self.a, v); self.cycles += 5; }

            0xE0 => { let a = imm!(); let v = bus.read(a); self.cmp(self.x, v); self.cycles += 2; }
            0xE4 => { let a = zp!(); let v = bus.read(a); self.cmp(self.x, v); self.cycles += 3; }
            0xEC => { let a = abs!(); let v = bus.read(a); self.cmp(self.x, v); self.cycles += 4; }

            0xC0 => { let a = imm!(); let v = bus.read(a); self.cmp(self.y, v); self.cycles += 2; }
            0xC4 => { let a = zp!(); let v = bus.read(a); self.cmp(self.y, v); self.cycles += 3; }
            0xCC => { let a = abs!(); let v = bus.read(a); self.cmp(self.y, v); self.cycles += 4; }

            // ── Logic ──
            0x29 => { let a = imm!(); self.a &= bus.read(a); self.set_zn(self.a); self.cycles += 2; }
            0x25 => { let a = zp!(); self.a &= bus.read(a); self.set_zn(self.a); self.cycles += 3; }
            0x35 => { let a = zpx!(); self.a &= bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0x2D => { let a = abs!(); self.a &= bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0x3D => { let a = abx!(); self.a &= bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0x39 => { let a = aby!(); self.a &= bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0x21 => { let a = izx!(); self.a &= bus.read(a); self.set_zn(self.a); self.cycles += 6; }
            0x31 => { let a = izy!(); self.a &= bus.read(a); self.set_zn(self.a); self.cycles += 5; }

            0x09 => { let a = imm!(); self.a |= bus.read(a); self.set_zn(self.a); self.cycles += 2; }
            0x05 => { let a = zp!(); self.a |= bus.read(a); self.set_zn(self.a); self.cycles += 3; }
            0x15 => { let a = zpx!(); self.a |= bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0x0D => { let a = abs!(); self.a |= bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0x1D => { let a = abx!(); self.a |= bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0x19 => { let a = aby!(); self.a |= bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0x01 => { let a = izx!(); self.a |= bus.read(a); self.set_zn(self.a); self.cycles += 6; }
            0x11 => { let a = izy!(); self.a |= bus.read(a); self.set_zn(self.a); self.cycles += 5; }

            0x49 => { let a = imm!(); self.a ^= bus.read(a); self.set_zn(self.a); self.cycles += 2; }
            0x45 => { let a = zp!(); self.a ^= bus.read(a); self.set_zn(self.a); self.cycles += 3; }
            0x55 => { let a = zpx!(); self.a ^= bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0x4D => { let a = abs!(); self.a ^= bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0x5D => { let a = abx!(); self.a ^= bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0x59 => { let a = aby!(); self.a ^= bus.read(a); self.set_zn(self.a); self.cycles += 4; }
            0x41 => { let a = izx!(); self.a ^= bus.read(a); self.set_zn(self.a); self.cycles += 6; }
            0x51 => { let a = izy!(); self.a ^= bus.read(a); self.set_zn(self.a); self.cycles += 5; }

            0x24 => { let a = zp!(); self.bit(bus.read(a)); self.cycles += 3; }
            0x2C => { let a = abs!(); self.bit(bus.read(a)); self.cycles += 4; }

            // ── Shifts ──
            0x0A => { self.a = self.asl(self.a); self.cycles += 2; }
            0x06 => { let a = zp!(); let v = self.asl(bus.read(a)); bus.write(a, v); self.cycles += 5; }
            0x16 => { let a = zpx!(); let v = self.asl(bus.read(a)); bus.write(a, v); self.cycles += 6; }
            0x0E => { let a = abs!(); let v = self.asl(bus.read(a)); bus.write(a, v); self.cycles += 6; }
            0x1E => { let a = abx_nopage!(); let v = self.asl(bus.read(a)); bus.write(a, v); self.cycles += 7; }

            0x4A => { self.a = self.lsr(self.a); self.cycles += 2; }
            0x46 => { let a = zp!(); let v = self.lsr(bus.read(a)); bus.write(a, v); self.cycles += 5; }
            0x56 => { let a = zpx!(); let v = self.lsr(bus.read(a)); bus.write(a, v); self.cycles += 6; }
            0x4E => { let a = abs!(); let v = self.lsr(bus.read(a)); bus.write(a, v); self.cycles += 6; }
            0x5E => { let a = abx_nopage!(); let v = self.lsr(bus.read(a)); bus.write(a, v); self.cycles += 7; }

            0x2A => { self.a = self.rol(self.a); self.cycles += 2; }
            0x26 => { let a = zp!(); let v = self.rol(bus.read(a)); bus.write(a, v); self.cycles += 5; }
            0x36 => { let a = zpx!(); let v = self.rol(bus.read(a)); bus.write(a, v); self.cycles += 6; }
            0x2E => { let a = abs!(); let v = self.rol(bus.read(a)); bus.write(a, v); self.cycles += 6; }
            0x3E => { let a = abx_nopage!(); let v = self.rol(bus.read(a)); bus.write(a, v); self.cycles += 7; }

            0x6A => { self.a = self.ror(self.a); self.cycles += 2; }
            0x66 => { let a = zp!(); let v = self.ror(bus.read(a)); bus.write(a, v); self.cycles += 5; }
            0x76 => { let a = zpx!(); let v = self.ror(bus.read(a)); bus.write(a, v); self.cycles += 6; }
            0x6E => { let a = abs!(); let v = self.ror(bus.read(a)); bus.write(a, v); self.cycles += 6; }
            0x7E => { let a = abx_nopage!(); let v = self.ror(bus.read(a)); bus.write(a, v); self.cycles += 7; }

            // ── Inc/Dec ──
            0xE6 => { let a = zp!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); self.set_zn(v); self.cycles += 5; }
            0xF6 => { let a = zpx!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); self.set_zn(v); self.cycles += 6; }
            0xEE => { let a = abs!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); self.set_zn(v); self.cycles += 6; }
            0xFE => { let a = abx_nopage!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); self.set_zn(v); self.cycles += 7; }

            0xC6 => { let a = zp!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); self.set_zn(v); self.cycles += 5; }
            0xD6 => { let a = zpx!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); self.set_zn(v); self.cycles += 6; }
            0xCE => { let a = abs!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); self.set_zn(v); self.cycles += 6; }
            0xDE => { let a = abx_nopage!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); self.set_zn(v); self.cycles += 7; }

            0xE8 => { self.x = self.x.wrapping_add(1); self.set_zn(self.x); self.cycles += 2; }
            0xC8 => { self.y = self.y.wrapping_add(1); self.set_zn(self.y); self.cycles += 2; }
            0xCA => { self.x = self.x.wrapping_sub(1); self.set_zn(self.x); self.cycles += 2; }
            0x88 => { self.y = self.y.wrapping_sub(1); self.set_zn(self.y); self.cycles += 2; }

            // ── Branches ──
            0x10 => { branch!(!self.n); }
            0x30 => { branch!(self.n); }
            0x50 => { branch!(!self.v); }
            0x70 => { branch!(self.v); }
            0x90 => { branch!(!self.c); }
            0xB0 => { branch!(self.c); }
            0xD0 => { branch!(!self.z); }
            0xF0 => { branch!(self.z); }

            // ── Jumps ──
            0x4C => { self.pc = abs!(); self.cycles += 3; }
            0x6C => {
                let a = abs!();
                self.pc = self.read16_bug(bus, a);
                self.cycles += 5;
            }
            0x20 => { // JSR
                let target = abs!();
                self.push16(bus, self.pc.wrapping_sub(1));
                self.pc = target;
                self.cycles += 6;
            }
            0x60 => { // RTS
                self.pc = self.pull16(bus).wrapping_add(1);
                self.cycles += 6;
            }
            0x40 => { // RTI
                let s = self.pull(bus);
                self.set_status(s);
                self.pc = self.pull16(bus);
                self.cycles += 6;
            }

            // ── Stack/Transfer ──
            0xAA => { self.x = self.a; self.set_zn(self.x); self.cycles += 2; }
            0x8A => { self.a = self.x; self.set_zn(self.a); self.cycles += 2; }
            0xA8 => { self.y = self.a; self.set_zn(self.y); self.cycles += 2; }
            0x98 => { self.a = self.y; self.set_zn(self.a); self.cycles += 2; }
            0x9A => { self.sp = self.x; self.cycles += 2; }
            0xBA => { self.x = self.sp; self.set_zn(self.x); self.cycles += 2; }
            0x48 => { self.push(bus, self.a); self.cycles += 3; }
            0x68 => { self.a = self.pull(bus); self.set_zn(self.a); self.cycles += 4; }
            0x08 => { self.push(bus, self.status() | 0x10); self.cycles += 3; }
            0x28 => { let s = self.pull(bus); self.set_status(s); self.cycles += 4; }

            // ── Flags ──
            0x18 => { self.c = false; self.cycles += 2; }
            0x38 => { self.c = true; self.cycles += 2; }
            0x58 => { self.i = false; self.cycles += 2; }
            0x78 => { self.i = true; self.cycles += 2; }
            0xB8 => { self.v = false; self.cycles += 2; }
            0xD8 => { self.d = false; self.cycles += 2; }
            0xF8 => { self.d = true; self.cycles += 2; }

            // ── NOP ──
            0xEA => { self.cycles += 2; }

            // ── BRK ──
            0x00 => {
                self.pc = self.pc.wrapping_add(1);
                self.push16(bus, self.pc);
                self.push(bus, self.status() | 0x10);
                self.i = true;
                self.pc = self.read16(bus, 0xFFFE);
                self.cycles += 7;
            }

            // ── Illegal NOPs (Contra may hit some) ──
            0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA => { self.cycles += 2; }
            0x04 | 0x44 | 0x64 => { self.pc = self.pc.wrapping_add(1); self.cycles += 3; }
            0x0C => { self.pc = self.pc.wrapping_add(2); self.cycles += 4; }
            0x14 | 0x34 | 0x54 | 0x74 | 0xD4 | 0xF4 => { self.pc = self.pc.wrapping_add(1); self.cycles += 4; }
            0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC => { self.pc = self.pc.wrapping_add(2); self.cycles += 4; }
            0x80 | 0x82 | 0x89 | 0xC2 | 0xE2 => { self.pc = self.pc.wrapping_add(1); self.cycles += 2; }

            // ── Illegal but used: LAX, SAX, DCP, ISB, SLO, RLA, SRE, RRA ──
            // LAX: LDA + LDX
            0xA7 => { let a = zp!(); self.a = bus.read(a); self.x = self.a; self.set_zn(self.a); self.cycles += 3; }
            0xB7 => { let a = zpy!(); self.a = bus.read(a); self.x = self.a; self.set_zn(self.a); self.cycles += 4; }
            0xAF => { let a = abs!(); self.a = bus.read(a); self.x = self.a; self.set_zn(self.a); self.cycles += 4; }
            0xBF => { let a = aby!(); self.a = bus.read(a); self.x = self.a; self.set_zn(self.a); self.cycles += 4; }
            0xA3 => { let a = izx!(); self.a = bus.read(a); self.x = self.a; self.set_zn(self.a); self.cycles += 6; }
            0xB3 => { let a = izy!(); self.a = bus.read(a); self.x = self.a; self.set_zn(self.a); self.cycles += 5; }

            // SAX: store A & X
            0x87 => { let a = zp!(); bus.write(a, self.a & self.x); self.cycles += 3; }
            0x97 => { let a = zpy!(); bus.write(a, self.a & self.x); self.cycles += 4; }
            0x8F => { let a = abs!(); bus.write(a, self.a & self.x); self.cycles += 4; }
            0x83 => { let a = izx!(); bus.write(a, self.a & self.x); self.cycles += 6; }

            // DCP: DEC + CMP
            0xC7 => { let a = zp!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); self.cmp(self.a, v); self.cycles += 5; }
            0xD7 => { let a = zpx!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); self.cmp(self.a, v); self.cycles += 6; }
            0xCF => { let a = abs!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); self.cmp(self.a, v); self.cycles += 6; }
            0xDF => { let a = abx_nopage!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); self.cmp(self.a, v); self.cycles += 7; }
            0xDB => { let a = aby_nopage!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); self.cmp(self.a, v); self.cycles += 7; }
            0xC3 => { let a = izx!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); self.cmp(self.a, v); self.cycles += 8; }
            0xD3 => { let a = izy_nopage!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); self.cmp(self.a, v); self.cycles += 8; }

            // ISB (ISC): INC + SBC
            0xE7 => { let a = zp!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); self.sbc(v); self.cycles += 5; }
            0xF7 => { let a = zpx!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); self.sbc(v); self.cycles += 6; }
            0xEF => { let a = abs!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); self.sbc(v); self.cycles += 6; }
            0xFF => { let a = abx_nopage!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); self.sbc(v); self.cycles += 7; }
            0xFB => { let a = aby_nopage!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); self.sbc(v); self.cycles += 7; }
            0xE3 => { let a = izx!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); self.sbc(v); self.cycles += 8; }
            0xF3 => { let a = izy_nopage!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); self.sbc(v); self.cycles += 8; }

            // SLO: ASL + ORA
            0x07 => { let a = zp!(); let v = self.asl(bus.read(a)); bus.write(a, v); self.a |= v; self.set_zn(self.a); self.cycles += 5; }
            0x17 => { let a = zpx!(); let v = self.asl(bus.read(a)); bus.write(a, v); self.a |= v; self.set_zn(self.a); self.cycles += 6; }
            0x0F => { let a = abs!(); let v = self.asl(bus.read(a)); bus.write(a, v); self.a |= v; self.set_zn(self.a); self.cycles += 6; }
            0x1F => { let a = abx_nopage!(); let v = self.asl(bus.read(a)); bus.write(a, v); self.a |= v; self.set_zn(self.a); self.cycles += 7; }
            0x1B => { let a = aby_nopage!(); let v = self.asl(bus.read(a)); bus.write(a, v); self.a |= v; self.set_zn(self.a); self.cycles += 7; }
            0x03 => { let a = izx!(); let v = self.asl(bus.read(a)); bus.write(a, v); self.a |= v; self.set_zn(self.a); self.cycles += 8; }
            0x13 => { let a = izy_nopage!(); let v = self.asl(bus.read(a)); bus.write(a, v); self.a |= v; self.set_zn(self.a); self.cycles += 8; }

            // RLA: ROL + AND
            0x27 => { let a = zp!(); let v = self.rol(bus.read(a)); bus.write(a, v); self.a &= v; self.set_zn(self.a); self.cycles += 5; }
            0x37 => { let a = zpx!(); let v = self.rol(bus.read(a)); bus.write(a, v); self.a &= v; self.set_zn(self.a); self.cycles += 6; }
            0x2F => { let a = abs!(); let v = self.rol(bus.read(a)); bus.write(a, v); self.a &= v; self.set_zn(self.a); self.cycles += 6; }
            0x3F => { let a = abx_nopage!(); let v = self.rol(bus.read(a)); bus.write(a, v); self.a &= v; self.set_zn(self.a); self.cycles += 7; }
            0x3B => { let a = aby_nopage!(); let v = self.rol(bus.read(a)); bus.write(a, v); self.a &= v; self.set_zn(self.a); self.cycles += 7; }
            0x23 => { let a = izx!(); let v = self.rol(bus.read(a)); bus.write(a, v); self.a &= v; self.set_zn(self.a); self.cycles += 8; }
            0x33 => { let a = izy_nopage!(); let v = self.rol(bus.read(a)); bus.write(a, v); self.a &= v; self.set_zn(self.a); self.cycles += 8; }

            // SRE: LSR + EOR
            0x47 => { let a = zp!(); let v = self.lsr(bus.read(a)); bus.write(a, v); self.a ^= v; self.set_zn(self.a); self.cycles += 5; }
            0x57 => { let a = zpx!(); let v = self.lsr(bus.read(a)); bus.write(a, v); self.a ^= v; self.set_zn(self.a); self.cycles += 6; }
            0x4F => { let a = abs!(); let v = self.lsr(bus.read(a)); bus.write(a, v); self.a ^= v; self.set_zn(self.a); self.cycles += 6; }
            0x5F => { let a = abx_nopage!(); let v = self.lsr(bus.read(a)); bus.write(a, v); self.a ^= v; self.set_zn(self.a); self.cycles += 7; }
            0x5B => { let a = aby_nopage!(); let v = self.lsr(bus.read(a)); bus.write(a, v); self.a ^= v; self.set_zn(self.a); self.cycles += 7; }
            0x43 => { let a = izx!(); let v = self.lsr(bus.read(a)); bus.write(a, v); self.a ^= v; self.set_zn(self.a); self.cycles += 8; }
            0x53 => { let a = izy_nopage!(); let v = self.lsr(bus.read(a)); bus.write(a, v); self.a ^= v; self.set_zn(self.a); self.cycles += 8; }

            // RRA: ROR + ADC
            0x67 => { let a = zp!(); let v = self.ror(bus.read(a)); bus.write(a, v); self.adc(v); self.cycles += 5; }
            0x77 => { let a = zpx!(); let v = self.ror(bus.read(a)); bus.write(a, v); self.adc(v); self.cycles += 6; }
            0x6F => { let a = abs!(); let v = self.ror(bus.read(a)); bus.write(a, v); self.adc(v); self.cycles += 6; }
            0x7F => { let a = abx_nopage!(); let v = self.ror(bus.read(a)); bus.write(a, v); self.adc(v); self.cycles += 7; }
            0x7B => { let a = aby_nopage!(); let v = self.ror(bus.read(a)); bus.write(a, v); self.adc(v); self.cycles += 7; }
            0x63 => { let a = izx!(); let v = self.ror(bus.read(a)); bus.write(a, v); self.adc(v); self.cycles += 8; }
            0x73 => { let a = izy_nopage!(); let v = self.ror(bus.read(a)); bus.write(a, v); self.adc(v); self.cycles += 8; }

            // EB = unofficial SBC immediate (same as E9)
            0xEB => { let a = imm!(); let v = bus.read(a); self.sbc(v); self.cycles += 2; }

            _ => {
                // Unknown opcode — treat as 1-byte NOP
                self.cycles += 2;
            }
        }

        (self.cycles - start) as u32
    }

    #[inline(always)]
    fn adc(&mut self, val: u8) {
        let sum = self.a as u16 + val as u16 + self.c as u16;
        self.c = sum > 0xFF;
        self.v = (!(self.a ^ val) & (self.a ^ sum as u8)) & 0x80 != 0;
        self.a = sum as u8;
        self.set_zn(self.a);
    }

    #[inline(always)]
    fn sbc(&mut self, val: u8) {
        self.adc(!val);
    }

    #[inline(always)]
    fn cmp(&mut self, reg: u8, val: u8) {
        let diff = reg.wrapping_sub(val);
        self.c = reg >= val;
        self.set_zn(diff);
    }

    #[inline(always)]
    fn bit(&mut self, val: u8) {
        self.z = self.a & val == 0;
        self.v = val & 0x40 != 0;
        self.n = val & 0x80 != 0;
    }

    #[inline(always)]
    fn asl(&mut self, val: u8) -> u8 {
        self.c = val & 0x80 != 0;
        let r = val << 1;
        self.set_zn(r);
        r
    }

    #[inline(always)]
    fn lsr(&mut self, val: u8) -> u8 {
        self.c = val & 1 != 0;
        let r = val >> 1;
        self.set_zn(r);
        r
    }

    #[inline(always)]
    fn rol(&mut self, val: u8) -> u8 {
        let old_c = self.c as u8;
        self.c = val & 0x80 != 0;
        let r = (val << 1) | old_c;
        self.set_zn(r);
        r
    }

    #[inline(always)]
    fn ror(&mut self, val: u8) -> u8 {
        let old_c = self.c as u8;
        self.c = val & 1 != 0;
        let r = (val >> 1) | (old_c << 7);
        self.set_zn(r);
        r
    }
}
