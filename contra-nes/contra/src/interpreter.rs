// Fallback single-instruction interpreter
//
// Used for addresses not covered by compiled blocks (rare).
// This is a simplified version of the main emulator's cpu.rs.

use crate::bus::Bus;
use crate::cpu_state::CpuState;

pub fn step(s: &mut CpuState, bus: &mut Bus) {
    let op = if s.pc >= 0x8000 { bus.cart.read_prg(s.pc) } else { bus.read(s.pc) };
    s.pc = s.pc.wrapping_add(1);

    macro_rules! read_pc {
        () => {{ let v = if s.pc >= 0x8000 { bus.cart.read_prg(s.pc) } else { bus.read(s.pc) }; s.pc = s.pc.wrapping_add(1); v }}
    }
    macro_rules! imm { () => {{ let v = s.pc; s.pc = s.pc.wrapping_add(1); v }} }
    macro_rules! zp { () => { read_pc!() as u16 } }
    macro_rules! zpx { () => { read_pc!().wrapping_add(s.x) as u16 } }
    macro_rules! zpy { () => { read_pc!().wrapping_add(s.y) as u16 } }
    macro_rules! abs { () => { { let lo = read_pc!() as u16; let hi = read_pc!() as u16; lo | (hi << 8) } } }
    macro_rules! abx { () => { { let b = abs!(); let a = b.wrapping_add(s.x as u16); if b & 0xFF00 != a & 0xFF00 { s.cycles += 1; } a } } }
    macro_rules! abx_np { () => { { let b = abs!(); b.wrapping_add(s.x as u16) } } }
    macro_rules! aby { () => { { let b = abs!(); let a = b.wrapping_add(s.y as u16); if b & 0xFF00 != a & 0xFF00 { s.cycles += 1; } a } } }
    macro_rules! aby_np { () => { { let b = abs!(); b.wrapping_add(s.y as u16) } } }
    macro_rules! izx { () => { { let ptr = read_pc!().wrapping_add(s.x); let lo = unsafe { *bus.ram.get_unchecked(ptr as usize) } as u16; let hi = unsafe { *bus.ram.get_unchecked(ptr.wrapping_add(1) as usize) } as u16; lo | (hi << 8) } } }
    macro_rules! izy { () => { { let ptr = read_pc!(); let lo = unsafe { *bus.ram.get_unchecked(ptr as usize) } as u16; let hi = unsafe { *bus.ram.get_unchecked(ptr.wrapping_add(1) as usize) } as u16; let b = lo | (hi << 8); let a = b.wrapping_add(s.y as u16); if b & 0xFF00 != a & 0xFF00 { s.cycles += 1; } a } } }
    macro_rules! izy_np { () => { { let ptr = read_pc!(); let lo = unsafe { *bus.ram.get_unchecked(ptr as usize) } as u16; let hi = unsafe { *bus.ram.get_unchecked(ptr.wrapping_add(1) as usize) } as u16; (lo | (hi << 8)).wrapping_add(s.y as u16) } } }
    macro_rules! branch { ($cond:expr) => {{ let offset = read_pc!() as i8; if $cond { let new_pc = s.pc.wrapping_add(offset as u16); s.cycles += 1; if s.pc & 0xFF00 != new_pc & 0xFF00 { s.cycles += 1; } s.pc = new_pc; } s.cycles += 2; }} }
    macro_rules! set_zn { ($v:expr) => { s.z = $v == 0; s.n = $v & 0x80 != 0; } }

    match op {
        0xA9 => { let a = imm!(); s.a = bus.read(a); set_zn!(s.a); s.cycles += 2; }
        0xA5 => { let a = zp!(); s.a = bus.read(a); set_zn!(s.a); s.cycles += 3; }
        0xB5 => { let a = zpx!(); s.a = bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0xAD => { let a = abs!(); s.a = bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0xBD => { let a = abx!(); s.a = bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0xB9 => { let a = aby!(); s.a = bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0xA1 => { let a = izx!(); s.a = bus.read(a); set_zn!(s.a); s.cycles += 6; }
        0xB1 => { let a = izy!(); s.a = bus.read(a); set_zn!(s.a); s.cycles += 5; }

        0xA2 => { let a = imm!(); s.x = bus.read(a); set_zn!(s.x); s.cycles += 2; }
        0xA6 => { let a = zp!(); s.x = bus.read(a); set_zn!(s.x); s.cycles += 3; }
        0xB6 => { let a = zpy!(); s.x = bus.read(a); set_zn!(s.x); s.cycles += 4; }
        0xAE => { let a = abs!(); s.x = bus.read(a); set_zn!(s.x); s.cycles += 4; }
        0xBE => { let a = aby!(); s.x = bus.read(a); set_zn!(s.x); s.cycles += 4; }

        0xA0 => { let a = imm!(); s.y = bus.read(a); set_zn!(s.y); s.cycles += 2; }
        0xA4 => { let a = zp!(); s.y = bus.read(a); set_zn!(s.y); s.cycles += 3; }
        0xB4 => { let a = zpx!(); s.y = bus.read(a); set_zn!(s.y); s.cycles += 4; }
        0xAC => { let a = abs!(); s.y = bus.read(a); set_zn!(s.y); s.cycles += 4; }
        0xBC => { let a = abx!(); s.y = bus.read(a); set_zn!(s.y); s.cycles += 4; }

        0x85 => { let a = zp!(); bus.write(a, s.a); s.cycles += 3; }
        0x95 => { let a = zpx!(); bus.write(a, s.a); s.cycles += 4; }
        0x8D => { let a = abs!(); bus.write(a, s.a); s.cycles += 4; }
        0x9D => { let a = abx_np!(); bus.write(a, s.a); s.cycles += 5; }
        0x99 => { let a = aby_np!(); bus.write(a, s.a); s.cycles += 5; }
        0x81 => { let a = izx!(); bus.write(a, s.a); s.cycles += 6; }
        0x91 => { let a = izy_np!(); bus.write(a, s.a); s.cycles += 6; }

        0x86 => { let a = zp!(); bus.write(a, s.x); s.cycles += 3; }
        0x96 => { let a = zpy!(); bus.write(a, s.x); s.cycles += 4; }
        0x8E => { let a = abs!(); bus.write(a, s.x); s.cycles += 4; }
        0x84 => { let a = zp!(); bus.write(a, s.y); s.cycles += 3; }
        0x94 => { let a = zpx!(); bus.write(a, s.y); s.cycles += 4; }
        0x8C => { let a = abs!(); bus.write(a, s.y); s.cycles += 4; }

        0x69 => { let a = imm!(); let v = bus.read(a); s.adc(v); s.cycles += 2; }
        0x65 => { let a = zp!(); let v = bus.read(a); s.adc(v); s.cycles += 3; }
        0x75 => { let a = zpx!(); let v = bus.read(a); s.adc(v); s.cycles += 4; }
        0x6D => { let a = abs!(); let v = bus.read(a); s.adc(v); s.cycles += 4; }
        0x7D => { let a = abx!(); let v = bus.read(a); s.adc(v); s.cycles += 4; }
        0x79 => { let a = aby!(); let v = bus.read(a); s.adc(v); s.cycles += 4; }
        0x61 => { let a = izx!(); let v = bus.read(a); s.adc(v); s.cycles += 6; }
        0x71 => { let a = izy!(); let v = bus.read(a); s.adc(v); s.cycles += 5; }

        0xE9 | 0xEB => { let a = imm!(); let v = bus.read(a); s.sbc(v); s.cycles += 2; }
        0xE5 => { let a = zp!(); let v = bus.read(a); s.sbc(v); s.cycles += 3; }
        0xF5 => { let a = zpx!(); let v = bus.read(a); s.sbc(v); s.cycles += 4; }
        0xED => { let a = abs!(); let v = bus.read(a); s.sbc(v); s.cycles += 4; }
        0xFD => { let a = abx!(); let v = bus.read(a); s.sbc(v); s.cycles += 4; }
        0xF9 => { let a = aby!(); let v = bus.read(a); s.sbc(v); s.cycles += 4; }
        0xE1 => { let a = izx!(); let v = bus.read(a); s.sbc(v); s.cycles += 6; }
        0xF1 => { let a = izy!(); let v = bus.read(a); s.sbc(v); s.cycles += 5; }

        0xC9 => { let a = imm!(); let v = bus.read(a); s.cmp_reg(s.a, v); s.cycles += 2; }
        0xC5 => { let a = zp!(); let v = bus.read(a); s.cmp_reg(s.a, v); s.cycles += 3; }
        0xD5 => { let a = zpx!(); let v = bus.read(a); s.cmp_reg(s.a, v); s.cycles += 4; }
        0xCD => { let a = abs!(); let v = bus.read(a); s.cmp_reg(s.a, v); s.cycles += 4; }
        0xDD => { let a = abx!(); let v = bus.read(a); s.cmp_reg(s.a, v); s.cycles += 4; }
        0xD9 => { let a = aby!(); let v = bus.read(a); s.cmp_reg(s.a, v); s.cycles += 4; }
        0xC1 => { let a = izx!(); let v = bus.read(a); s.cmp_reg(s.a, v); s.cycles += 6; }
        0xD1 => { let a = izy!(); let v = bus.read(a); s.cmp_reg(s.a, v); s.cycles += 5; }

        0xE0 => { let a = imm!(); let v = bus.read(a); s.cmp_reg(s.x, v); s.cycles += 2; }
        0xE4 => { let a = zp!(); let v = bus.read(a); s.cmp_reg(s.x, v); s.cycles += 3; }
        0xEC => { let a = abs!(); let v = bus.read(a); s.cmp_reg(s.x, v); s.cycles += 4; }
        0xC0 => { let a = imm!(); let v = bus.read(a); s.cmp_reg(s.y, v); s.cycles += 2; }
        0xC4 => { let a = zp!(); let v = bus.read(a); s.cmp_reg(s.y, v); s.cycles += 3; }
        0xCC => { let a = abs!(); let v = bus.read(a); s.cmp_reg(s.y, v); s.cycles += 4; }

        0x29 => { let a = imm!(); s.a &= bus.read(a); set_zn!(s.a); s.cycles += 2; }
        0x25 => { let a = zp!(); s.a &= bus.read(a); set_zn!(s.a); s.cycles += 3; }
        0x35 => { let a = zpx!(); s.a &= bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0x2D => { let a = abs!(); s.a &= bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0x3D => { let a = abx!(); s.a &= bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0x39 => { let a = aby!(); s.a &= bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0x21 => { let a = izx!(); s.a &= bus.read(a); set_zn!(s.a); s.cycles += 6; }
        0x31 => { let a = izy!(); s.a &= bus.read(a); set_zn!(s.a); s.cycles += 5; }

        0x09 => { let a = imm!(); s.a |= bus.read(a); set_zn!(s.a); s.cycles += 2; }
        0x05 => { let a = zp!(); s.a |= bus.read(a); set_zn!(s.a); s.cycles += 3; }
        0x15 => { let a = zpx!(); s.a |= bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0x0D => { let a = abs!(); s.a |= bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0x1D => { let a = abx!(); s.a |= bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0x19 => { let a = aby!(); s.a |= bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0x01 => { let a = izx!(); s.a |= bus.read(a); set_zn!(s.a); s.cycles += 6; }
        0x11 => { let a = izy!(); s.a |= bus.read(a); set_zn!(s.a); s.cycles += 5; }

        0x49 => { let a = imm!(); s.a ^= bus.read(a); set_zn!(s.a); s.cycles += 2; }
        0x45 => { let a = zp!(); s.a ^= bus.read(a); set_zn!(s.a); s.cycles += 3; }
        0x55 => { let a = zpx!(); s.a ^= bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0x4D => { let a = abs!(); s.a ^= bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0x5D => { let a = abx!(); s.a ^= bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0x59 => { let a = aby!(); s.a ^= bus.read(a); set_zn!(s.a); s.cycles += 4; }
        0x41 => { let a = izx!(); s.a ^= bus.read(a); set_zn!(s.a); s.cycles += 6; }
        0x51 => { let a = izy!(); s.a ^= bus.read(a); set_zn!(s.a); s.cycles += 5; }

        0x24 => { let a = zp!(); let v = bus.read(a); s.z = s.a & v == 0; s.v = v & 0x40 != 0; s.n = v & 0x80 != 0; s.cycles += 3; }
        0x2C => { let a = abs!(); let v = bus.read(a); s.z = s.a & v == 0; s.v = v & 0x40 != 0; s.n = v & 0x80 != 0; s.cycles += 4; }

        0x0A => { s.c = s.a & 0x80 != 0; s.a <<= 1; set_zn!(s.a); s.cycles += 2; }
        0x06 => { let a = zp!(); let mut v = bus.read(a); s.c = v & 0x80 != 0; v <<= 1; set_zn!(v); bus.write(a, v); s.cycles += 5; }
        0x16 => { let a = zpx!(); let mut v = bus.read(a); s.c = v & 0x80 != 0; v <<= 1; set_zn!(v); bus.write(a, v); s.cycles += 6; }
        0x0E => { let a = abs!(); let mut v = bus.read(a); s.c = v & 0x80 != 0; v <<= 1; set_zn!(v); bus.write(a, v); s.cycles += 6; }
        0x1E => { let a = abx_np!(); let mut v = bus.read(a); s.c = v & 0x80 != 0; v <<= 1; set_zn!(v); bus.write(a, v); s.cycles += 7; }

        0x4A => { s.c = s.a & 1 != 0; s.a >>= 1; set_zn!(s.a); s.cycles += 2; }
        0x46 => { let a = zp!(); let mut v = bus.read(a); s.c = v & 1 != 0; v >>= 1; set_zn!(v); bus.write(a, v); s.cycles += 5; }
        0x56 => { let a = zpx!(); let mut v = bus.read(a); s.c = v & 1 != 0; v >>= 1; set_zn!(v); bus.write(a, v); s.cycles += 6; }
        0x4E => { let a = abs!(); let mut v = bus.read(a); s.c = v & 1 != 0; v >>= 1; set_zn!(v); bus.write(a, v); s.cycles += 6; }
        0x5E => { let a = abx_np!(); let mut v = bus.read(a); s.c = v & 1 != 0; v >>= 1; set_zn!(v); bus.write(a, v); s.cycles += 7; }

        0x2A => { let oc = s.c as u8; s.c = s.a & 0x80 != 0; s.a = (s.a << 1) | oc; set_zn!(s.a); s.cycles += 2; }
        0x26 => { let a = zp!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 0x80 != 0; v = (v << 1) | oc; set_zn!(v); bus.write(a, v); s.cycles += 5; }
        0x36 => { let a = zpx!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 0x80 != 0; v = (v << 1) | oc; set_zn!(v); bus.write(a, v); s.cycles += 6; }
        0x2E => { let a = abs!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 0x80 != 0; v = (v << 1) | oc; set_zn!(v); bus.write(a, v); s.cycles += 6; }
        0x3E => { let a = abx_np!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 0x80 != 0; v = (v << 1) | oc; set_zn!(v); bus.write(a, v); s.cycles += 7; }

        0x6A => { let oc = s.c as u8; s.c = s.a & 1 != 0; s.a = (s.a >> 1) | (oc << 7); set_zn!(s.a); s.cycles += 2; }
        0x66 => { let a = zp!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 1 != 0; v = (v >> 1) | (oc << 7); set_zn!(v); bus.write(a, v); s.cycles += 5; }
        0x76 => { let a = zpx!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 1 != 0; v = (v >> 1) | (oc << 7); set_zn!(v); bus.write(a, v); s.cycles += 6; }
        0x6E => { let a = abs!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 1 != 0; v = (v >> 1) | (oc << 7); set_zn!(v); bus.write(a, v); s.cycles += 6; }
        0x7E => { let a = abx_np!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 1 != 0; v = (v >> 1) | (oc << 7); set_zn!(v); bus.write(a, v); s.cycles += 7; }

        0xE6 => { let a = zp!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); set_zn!(v); s.cycles += 5; }
        0xF6 => { let a = zpx!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); set_zn!(v); s.cycles += 6; }
        0xEE => { let a = abs!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); set_zn!(v); s.cycles += 6; }
        0xFE => { let a = abx_np!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); set_zn!(v); s.cycles += 7; }

        0xC6 => { let a = zp!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); set_zn!(v); s.cycles += 5; }
        0xD6 => { let a = zpx!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); set_zn!(v); s.cycles += 6; }
        0xCE => { let a = abs!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); set_zn!(v); s.cycles += 6; }
        0xDE => { let a = abx_np!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); set_zn!(v); s.cycles += 7; }

        0xE8 => { s.x = s.x.wrapping_add(1); set_zn!(s.x); s.cycles += 2; }
        0xC8 => { s.y = s.y.wrapping_add(1); set_zn!(s.y); s.cycles += 2; }
        0xCA => { s.x = s.x.wrapping_sub(1); set_zn!(s.x); s.cycles += 2; }
        0x88 => { s.y = s.y.wrapping_sub(1); set_zn!(s.y); s.cycles += 2; }

        0x10 => { branch!(!s.n); }
        0x30 => { branch!(s.n); }
        0x50 => { branch!(!s.v); }
        0x70 => { branch!(s.v); }
        0x90 => { branch!(!s.c); }
        0xB0 => { branch!(s.c); }
        0xD0 => { branch!(!s.z); }
        0xF0 => { branch!(s.z); }

        0x4C => { s.pc = abs!(); s.cycles += 3; }
        0x6C => { let a = abs!(); let lo = bus.read(a) as u16; let hi = bus.read((a & 0xFF00) | ((a+1) & 0x00FF)) as u16; s.pc = lo | (hi << 8); s.cycles += 5; }
        0x20 => { let target = abs!(); s.push16(bus, s.pc.wrapping_sub(1)); s.pc = target; s.cycles += 6; }
        0x60 => { s.pc = s.pull16(bus).wrapping_add(1); s.cycles += 6; }
        0x40 => { let st = s.pull(bus); s.set_status(st); s.pc = s.pull16(bus); s.cycles += 6; }

        0xAA => { s.x = s.a; set_zn!(s.x); s.cycles += 2; }
        0x8A => { s.a = s.x; set_zn!(s.a); s.cycles += 2; }
        0xA8 => { s.y = s.a; set_zn!(s.y); s.cycles += 2; }
        0x98 => { s.a = s.y; set_zn!(s.a); s.cycles += 2; }
        0x9A => { s.sp = s.x; s.cycles += 2; }
        0xBA => { s.x = s.sp; set_zn!(s.x); s.cycles += 2; }
        0x48 => { s.push(bus, s.a); s.cycles += 3; }
        0x68 => { s.a = s.pull(bus); set_zn!(s.a); s.cycles += 4; }
        0x08 => { s.push(bus, s.status() | 0x10); s.cycles += 3; }
        0x28 => { let st = s.pull(bus); s.set_status(st); s.cycles += 4; }

        0x18 => { s.c = false; s.cycles += 2; }
        0x38 => { s.c = true; s.cycles += 2; }
        0x58 => { s.i = false; s.cycles += 2; }
        0x78 => { s.i = true; s.cycles += 2; }
        0xB8 => { s.v = false; s.cycles += 2; }
        0xD8 => { s.d = false; s.cycles += 2; }
        0xF8 => { s.d = true; s.cycles += 2; }

        0xEA => { s.cycles += 2; }
        0x00 => { s.pc = s.pc.wrapping_add(1); s.push16(bus, s.pc); s.push(bus, s.status() | 0x10); s.i = true; let lo = bus.read(0xFFFE) as u16; let hi = bus.read(0xFFFF) as u16; s.pc = lo | (hi << 8); s.cycles += 7; }

        0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA => { s.cycles += 2; }
        0x04 | 0x44 | 0x64 => { s.pc = s.pc.wrapping_add(1); s.cycles += 3; }
        0x0C => { s.pc = s.pc.wrapping_add(2); s.cycles += 4; }
        0x14 | 0x34 | 0x54 | 0x74 | 0xD4 | 0xF4 => { s.pc = s.pc.wrapping_add(1); s.cycles += 4; }
        0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC => { s.pc = s.pc.wrapping_add(2); s.cycles += 4; }
        0x80 | 0x82 | 0x89 | 0xC2 | 0xE2 => { s.pc = s.pc.wrapping_add(1); s.cycles += 2; }

        // Illegal opcodes used by some games
        0xA7 => { let a = zp!(); s.a = bus.read(a); s.x = s.a; set_zn!(s.a); s.cycles += 3; }
        0xB7 => { let a = zpy!(); s.a = bus.read(a); s.x = s.a; set_zn!(s.a); s.cycles += 4; }
        0xAF => { let a = abs!(); s.a = bus.read(a); s.x = s.a; set_zn!(s.a); s.cycles += 4; }
        0xBF => { let a = aby!(); s.a = bus.read(a); s.x = s.a; set_zn!(s.a); s.cycles += 4; }
        0xA3 => { let a = izx!(); s.a = bus.read(a); s.x = s.a; set_zn!(s.a); s.cycles += 6; }
        0xB3 => { let a = izy!(); s.a = bus.read(a); s.x = s.a; set_zn!(s.a); s.cycles += 5; }

        0x87 => { let a = zp!(); bus.write(a, s.a & s.x); s.cycles += 3; }
        0x97 => { let a = zpy!(); bus.write(a, s.a & s.x); s.cycles += 4; }
        0x8F => { let a = abs!(); bus.write(a, s.a & s.x); s.cycles += 4; }
        0x83 => { let a = izx!(); bus.write(a, s.a & s.x); s.cycles += 6; }

        0xC7 => { let a = zp!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); s.cmp_reg(s.a, v); s.cycles += 5; }
        0xD7 => { let a = zpx!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); s.cmp_reg(s.a, v); s.cycles += 6; }
        0xCF => { let a = abs!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); s.cmp_reg(s.a, v); s.cycles += 6; }
        0xDF => { let a = abx_np!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); s.cmp_reg(s.a, v); s.cycles += 7; }
        0xDB => { let a = aby_np!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); s.cmp_reg(s.a, v); s.cycles += 7; }
        0xC3 => { let a = izx!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); s.cmp_reg(s.a, v); s.cycles += 8; }
        0xD3 => { let a = izy_np!(); let v = bus.read(a).wrapping_sub(1); bus.write(a, v); s.cmp_reg(s.a, v); s.cycles += 8; }

        0xE7 => { let a = zp!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); s.sbc(v); s.cycles += 5; }
        0xF7 => { let a = zpx!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); s.sbc(v); s.cycles += 6; }
        0xEF => { let a = abs!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); s.sbc(v); s.cycles += 6; }
        0xFF => { let a = abx_np!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); s.sbc(v); s.cycles += 7; }
        0xFB => { let a = aby_np!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); s.sbc(v); s.cycles += 7; }
        0xE3 => { let a = izx!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); s.sbc(v); s.cycles += 8; }
        0xF3 => { let a = izy_np!(); let v = bus.read(a).wrapping_add(1); bus.write(a, v); s.sbc(v); s.cycles += 8; }

        0x07 => { let a = zp!(); let mut v = bus.read(a); s.c = v & 0x80 != 0; v <<= 1; bus.write(a, v); s.a |= v; set_zn!(s.a); s.cycles += 5; }
        0x17 => { let a = zpx!(); let mut v = bus.read(a); s.c = v & 0x80 != 0; v <<= 1; bus.write(a, v); s.a |= v; set_zn!(s.a); s.cycles += 6; }
        0x0F => { let a = abs!(); let mut v = bus.read(a); s.c = v & 0x80 != 0; v <<= 1; bus.write(a, v); s.a |= v; set_zn!(s.a); s.cycles += 6; }
        0x1F => { let a = abx_np!(); let mut v = bus.read(a); s.c = v & 0x80 != 0; v <<= 1; bus.write(a, v); s.a |= v; set_zn!(s.a); s.cycles += 7; }
        0x1B => { let a = aby_np!(); let mut v = bus.read(a); s.c = v & 0x80 != 0; v <<= 1; bus.write(a, v); s.a |= v; set_zn!(s.a); s.cycles += 7; }
        0x03 => { let a = izx!(); let mut v = bus.read(a); s.c = v & 0x80 != 0; v <<= 1; bus.write(a, v); s.a |= v; set_zn!(s.a); s.cycles += 8; }
        0x13 => { let a = izy_np!(); let mut v = bus.read(a); s.c = v & 0x80 != 0; v <<= 1; bus.write(a, v); s.a |= v; set_zn!(s.a); s.cycles += 8; }

        0x27 => { let a = zp!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 0x80 != 0; v = (v << 1) | oc; bus.write(a, v); s.a &= v; set_zn!(s.a); s.cycles += 5; }
        0x37 => { let a = zpx!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 0x80 != 0; v = (v << 1) | oc; bus.write(a, v); s.a &= v; set_zn!(s.a); s.cycles += 6; }
        0x2F => { let a = abs!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 0x80 != 0; v = (v << 1) | oc; bus.write(a, v); s.a &= v; set_zn!(s.a); s.cycles += 6; }
        0x3F => { let a = abx_np!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 0x80 != 0; v = (v << 1) | oc; bus.write(a, v); s.a &= v; set_zn!(s.a); s.cycles += 7; }
        0x3B => { let a = aby_np!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 0x80 != 0; v = (v << 1) | oc; bus.write(a, v); s.a &= v; set_zn!(s.a); s.cycles += 7; }
        0x23 => { let a = izx!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 0x80 != 0; v = (v << 1) | oc; bus.write(a, v); s.a &= v; set_zn!(s.a); s.cycles += 8; }
        0x33 => { let a = izy_np!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 0x80 != 0; v = (v << 1) | oc; bus.write(a, v); s.a &= v; set_zn!(s.a); s.cycles += 8; }

        0x47 => { let a = zp!(); let mut v = bus.read(a); s.c = v & 1 != 0; v >>= 1; bus.write(a, v); s.a ^= v; set_zn!(s.a); s.cycles += 5; }
        0x57 => { let a = zpx!(); let mut v = bus.read(a); s.c = v & 1 != 0; v >>= 1; bus.write(a, v); s.a ^= v; set_zn!(s.a); s.cycles += 6; }
        0x4F => { let a = abs!(); let mut v = bus.read(a); s.c = v & 1 != 0; v >>= 1; bus.write(a, v); s.a ^= v; set_zn!(s.a); s.cycles += 6; }
        0x5F => { let a = abx_np!(); let mut v = bus.read(a); s.c = v & 1 != 0; v >>= 1; bus.write(a, v); s.a ^= v; set_zn!(s.a); s.cycles += 7; }
        0x5B => { let a = aby_np!(); let mut v = bus.read(a); s.c = v & 1 != 0; v >>= 1; bus.write(a, v); s.a ^= v; set_zn!(s.a); s.cycles += 7; }
        0x43 => { let a = izx!(); let mut v = bus.read(a); s.c = v & 1 != 0; v >>= 1; bus.write(a, v); s.a ^= v; set_zn!(s.a); s.cycles += 8; }
        0x53 => { let a = izy_np!(); let mut v = bus.read(a); s.c = v & 1 != 0; v >>= 1; bus.write(a, v); s.a ^= v; set_zn!(s.a); s.cycles += 8; }

        0x67 => { let a = zp!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 1 != 0; v = (v >> 1) | (oc << 7); bus.write(a, v); s.adc(v); s.cycles += 5; }
        0x77 => { let a = zpx!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 1 != 0; v = (v >> 1) | (oc << 7); bus.write(a, v); s.adc(v); s.cycles += 6; }
        0x6F => { let a = abs!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 1 != 0; v = (v >> 1) | (oc << 7); bus.write(a, v); s.adc(v); s.cycles += 6; }
        0x7F => { let a = abx_np!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 1 != 0; v = (v >> 1) | (oc << 7); bus.write(a, v); s.adc(v); s.cycles += 7; }
        0x7B => { let a = aby_np!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 1 != 0; v = (v >> 1) | (oc << 7); bus.write(a, v); s.adc(v); s.cycles += 7; }
        0x63 => { let a = izx!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 1 != 0; v = (v >> 1) | (oc << 7); bus.write(a, v); s.adc(v); s.cycles += 8; }
        0x73 => { let a = izy_np!(); let mut v = bus.read(a); let oc = s.c as u8; s.c = v & 1 != 0; v = (v >> 1) | (oc << 7); bus.write(a, v); s.adc(v); s.cycles += 8; }

        _ => { s.cycles += 2; }
    }
}
