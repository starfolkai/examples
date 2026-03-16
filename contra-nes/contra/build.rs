// build.rs — Static recompiler for Contra NES (6502 → Rust)
//
// Reads the iNES ROM at build time, performs recursive descent disassembly
// across all 8 PRG banks, identifies basic blocks, and generates native
// Rust code for each block. The generated code eliminates opcode fetch,
// decode, and dispatch overhead — the hot path becomes straight-line Rust.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

const PRG_BANK_SIZE: usize = 16384;

// ── Opcode metadata ──

#[derive(Clone, Copy, PartialEq)]
enum InstKind {
    Normal,
    Branch,
    JmpAbs,
    JmpInd,
    Jsr,
    Rts,
    Rti,
    Brk,
}

#[derive(Clone, Copy, PartialEq)]
enum AddrMode {
    Impl, Acc, Imm, Zp, ZpX, ZpY, Abs, AbsX, AbsY, IzX, IzY, Rel, Ind,
}

struct OpcodeInfo {
    len: u8,
    cycles: u8,
    kind: InstKind,
    mode: AddrMode,
    name: &'static str,
}

fn opcode_info(op: u8) -> OpcodeInfo {
    use AddrMode::*;
    use InstKind::*;
    macro_rules! oi {
        ($len:expr, $cyc:expr, $kind:expr, $mode:expr, $name:expr) => {
            OpcodeInfo { len: $len, cycles: $cyc, kind: $kind, mode: $mode, name: $name }
        }
    }
    match op {
        // ── Load/Store ──
        0xA9 => oi!(2, 2, Normal, Imm, "LDA"),
        0xA5 => oi!(2, 3, Normal, Zp, "LDA"),
        0xB5 => oi!(2, 4, Normal, ZpX, "LDA"),
        0xAD => oi!(3, 4, Normal, Abs, "LDA"),
        0xBD => oi!(3, 4, Normal, AbsX, "LDA"),
        0xB9 => oi!(3, 4, Normal, AbsY, "LDA"),
        0xA1 => oi!(2, 6, Normal, IzX, "LDA"),
        0xB1 => oi!(2, 5, Normal, IzY, "LDA"),

        0xA2 => oi!(2, 2, Normal, Imm, "LDX"),
        0xA6 => oi!(2, 3, Normal, Zp, "LDX"),
        0xB6 => oi!(2, 4, Normal, ZpY, "LDX"),
        0xAE => oi!(3, 4, Normal, Abs, "LDX"),
        0xBE => oi!(3, 4, Normal, AbsY, "LDX"),

        0xA0 => oi!(2, 2, Normal, Imm, "LDY"),
        0xA4 => oi!(2, 3, Normal, Zp, "LDY"),
        0xB4 => oi!(2, 4, Normal, ZpX, "LDY"),
        0xAC => oi!(3, 4, Normal, Abs, "LDY"),
        0xBC => oi!(3, 4, Normal, AbsX, "LDY"),

        0x85 => oi!(2, 3, Normal, Zp, "STA"),
        0x95 => oi!(2, 4, Normal, ZpX, "STA"),
        0x8D => oi!(3, 4, Normal, Abs, "STA"),
        0x9D => oi!(3, 5, Normal, AbsX, "STA"),
        0x99 => oi!(3, 5, Normal, AbsY, "STA"),
        0x81 => oi!(2, 6, Normal, IzX, "STA"),
        0x91 => oi!(2, 6, Normal, IzY, "STA"),

        0x86 => oi!(2, 3, Normal, Zp, "STX"),
        0x96 => oi!(2, 4, Normal, ZpY, "STX"),
        0x8E => oi!(3, 4, Normal, Abs, "STX"),

        0x84 => oi!(2, 3, Normal, Zp, "STY"),
        0x94 => oi!(2, 4, Normal, ZpX, "STY"),
        0x8C => oi!(3, 4, Normal, Abs, "STY"),

        // ── Arithmetic ──
        0x69 => oi!(2, 2, Normal, Imm, "ADC"),
        0x65 => oi!(2, 3, Normal, Zp, "ADC"),
        0x75 => oi!(2, 4, Normal, ZpX, "ADC"),
        0x6D => oi!(3, 4, Normal, Abs, "ADC"),
        0x7D => oi!(3, 4, Normal, AbsX, "ADC"),
        0x79 => oi!(3, 4, Normal, AbsY, "ADC"),
        0x61 => oi!(2, 6, Normal, IzX, "ADC"),
        0x71 => oi!(2, 5, Normal, IzY, "ADC"),

        0xE9 => oi!(2, 2, Normal, Imm, "SBC"),
        0xE5 => oi!(2, 3, Normal, Zp, "SBC"),
        0xF5 => oi!(2, 4, Normal, ZpX, "SBC"),
        0xED => oi!(3, 4, Normal, Abs, "SBC"),
        0xFD => oi!(3, 4, Normal, AbsX, "SBC"),
        0xF9 => oi!(3, 4, Normal, AbsY, "SBC"),
        0xE1 => oi!(2, 6, Normal, IzX, "SBC"),
        0xF1 => oi!(2, 5, Normal, IzY, "SBC"),

        0xC9 => oi!(2, 2, Normal, Imm, "CMP"),
        0xC5 => oi!(2, 3, Normal, Zp, "CMP"),
        0xD5 => oi!(2, 4, Normal, ZpX, "CMP"),
        0xCD => oi!(3, 4, Normal, Abs, "CMP"),
        0xDD => oi!(3, 4, Normal, AbsX, "CMP"),
        0xD9 => oi!(3, 4, Normal, AbsY, "CMP"),
        0xC1 => oi!(2, 6, Normal, IzX, "CMP"),
        0xD1 => oi!(2, 5, Normal, IzY, "CMP"),

        0xE0 => oi!(2, 2, Normal, Imm, "CPX"),
        0xE4 => oi!(2, 3, Normal, Zp, "CPX"),
        0xEC => oi!(3, 4, Normal, Abs, "CPX"),

        0xC0 => oi!(2, 2, Normal, Imm, "CPY"),
        0xC4 => oi!(2, 3, Normal, Zp, "CPY"),
        0xCC => oi!(3, 4, Normal, Abs, "CPY"),

        // ── Logic ──
        0x29 => oi!(2, 2, Normal, Imm, "AND"),
        0x25 => oi!(2, 3, Normal, Zp, "AND"),
        0x35 => oi!(2, 4, Normal, ZpX, "AND"),
        0x2D => oi!(3, 4, Normal, Abs, "AND"),
        0x3D => oi!(3, 4, Normal, AbsX, "AND"),
        0x39 => oi!(3, 4, Normal, AbsY, "AND"),
        0x21 => oi!(2, 6, Normal, IzX, "AND"),
        0x31 => oi!(2, 5, Normal, IzY, "AND"),

        0x09 => oi!(2, 2, Normal, Imm, "ORA"),
        0x05 => oi!(2, 3, Normal, Zp, "ORA"),
        0x15 => oi!(2, 4, Normal, ZpX, "ORA"),
        0x0D => oi!(3, 4, Normal, Abs, "ORA"),
        0x1D => oi!(3, 4, Normal, AbsX, "ORA"),
        0x19 => oi!(3, 4, Normal, AbsY, "ORA"),
        0x01 => oi!(2, 6, Normal, IzX, "ORA"),
        0x11 => oi!(2, 5, Normal, IzY, "ORA"),

        0x49 => oi!(2, 2, Normal, Imm, "EOR"),
        0x45 => oi!(2, 3, Normal, Zp, "EOR"),
        0x55 => oi!(2, 4, Normal, ZpX, "EOR"),
        0x4D => oi!(3, 4, Normal, Abs, "EOR"),
        0x5D => oi!(3, 4, Normal, AbsX, "EOR"),
        0x59 => oi!(3, 4, Normal, AbsY, "EOR"),
        0x41 => oi!(2, 6, Normal, IzX, "EOR"),
        0x51 => oi!(2, 5, Normal, IzY, "EOR"),

        0x24 => oi!(2, 3, Normal, Zp, "BIT"),
        0x2C => oi!(3, 4, Normal, Abs, "BIT"),

        // ── Shifts (memory) ──
        0x0A => oi!(1, 2, Normal, Acc, "ASL"),
        0x06 => oi!(2, 5, Normal, Zp, "ASL"),
        0x16 => oi!(2, 6, Normal, ZpX, "ASL"),
        0x0E => oi!(3, 6, Normal, Abs, "ASL"),
        0x1E => oi!(3, 7, Normal, AbsX, "ASL"),

        0x4A => oi!(1, 2, Normal, Acc, "LSR"),
        0x46 => oi!(2, 5, Normal, Zp, "LSR"),
        0x56 => oi!(2, 6, Normal, ZpX, "LSR"),
        0x4E => oi!(3, 6, Normal, Abs, "LSR"),
        0x5E => oi!(3, 7, Normal, AbsX, "LSR"),

        0x2A => oi!(1, 2, Normal, Acc, "ROL"),
        0x26 => oi!(2, 5, Normal, Zp, "ROL"),
        0x36 => oi!(2, 6, Normal, ZpX, "ROL"),
        0x2E => oi!(3, 6, Normal, Abs, "ROL"),
        0x3E => oi!(3, 7, Normal, AbsX, "ROL"),

        0x6A => oi!(1, 2, Normal, Acc, "ROR"),
        0x66 => oi!(2, 5, Normal, Zp, "ROR"),
        0x76 => oi!(2, 6, Normal, ZpX, "ROR"),
        0x6E => oi!(3, 6, Normal, Abs, "ROR"),
        0x7E => oi!(3, 7, Normal, AbsX, "ROR"),

        // ── Inc/Dec ──
        0xE6 => oi!(2, 5, Normal, Zp, "INC"),
        0xF6 => oi!(2, 6, Normal, ZpX, "INC"),
        0xEE => oi!(3, 6, Normal, Abs, "INC"),
        0xFE => oi!(3, 7, Normal, AbsX, "INC"),

        0xC6 => oi!(2, 5, Normal, Zp, "DEC"),
        0xD6 => oi!(2, 6, Normal, ZpX, "DEC"),
        0xCE => oi!(3, 6, Normal, Abs, "DEC"),
        0xDE => oi!(3, 7, Normal, AbsX, "DEC"),

        0xE8 => oi!(1, 2, Normal, Impl, "INX"),
        0xC8 => oi!(1, 2, Normal, Impl, "INY"),
        0xCA => oi!(1, 2, Normal, Impl, "DEX"),
        0x88 => oi!(1, 2, Normal, Impl, "DEY"),

        // ── Branches ──
        0x10 => oi!(2, 2, Branch, Rel, "BPL"),
        0x30 => oi!(2, 2, Branch, Rel, "BMI"),
        0x50 => oi!(2, 2, Branch, Rel, "BVC"),
        0x70 => oi!(2, 2, Branch, Rel, "BVS"),
        0x90 => oi!(2, 2, Branch, Rel, "BCC"),
        0xB0 => oi!(2, 2, Branch, Rel, "BCS"),
        0xD0 => oi!(2, 2, Branch, Rel, "BNE"),
        0xF0 => oi!(2, 2, Branch, Rel, "BEQ"),

        // ── Jumps ──
        0x4C => oi!(3, 3, JmpAbs, Abs, "JMP"),
        0x6C => oi!(3, 5, JmpInd, Ind, "JMP"),
        0x20 => oi!(3, 6, Jsr, Abs, "JSR"),
        0x60 => oi!(1, 6, Rts, Impl, "RTS"),
        0x40 => oi!(1, 6, Rti, Impl, "RTI"),

        // ── Stack/Transfer ──
        0xAA => oi!(1, 2, Normal, Impl, "TAX"),
        0x8A => oi!(1, 2, Normal, Impl, "TXA"),
        0xA8 => oi!(1, 2, Normal, Impl, "TAY"),
        0x98 => oi!(1, 2, Normal, Impl, "TYA"),
        0x9A => oi!(1, 2, Normal, Impl, "TXS"),
        0xBA => oi!(1, 2, Normal, Impl, "TSX"),
        0x48 => oi!(1, 3, Normal, Impl, "PHA"),
        0x68 => oi!(1, 4, Normal, Impl, "PLA"),
        0x08 => oi!(1, 3, Normal, Impl, "PHP"),
        0x28 => oi!(1, 4, Normal, Impl, "PLP"),

        // ── Flags ──
        0x18 => oi!(1, 2, Normal, Impl, "CLC"),
        0x38 => oi!(1, 2, Normal, Impl, "SEC"),
        0x58 => oi!(1, 2, Normal, Impl, "CLI"),
        0x78 => oi!(1, 2, Normal, Impl, "SEI"),
        0xB8 => oi!(1, 2, Normal, Impl, "CLV"),
        0xD8 => oi!(1, 2, Normal, Impl, "CLD"),
        0xF8 => oi!(1, 2, Normal, Impl, "SED"),

        // ── NOP ──
        0xEA => oi!(1, 2, Normal, Impl, "NOP"),

        // ── BRK ──
        0x00 => oi!(1, 7, Brk, Impl, "BRK"),

        // ── Illegal NOPs ──
        0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA => oi!(1, 2, Normal, Impl, "NOP"),
        0x04 | 0x44 | 0x64 => oi!(2, 3, Normal, Zp, "DOP"),
        0x0C => oi!(3, 4, Normal, Abs, "TOP"),
        0x14 | 0x34 | 0x54 | 0x74 | 0xD4 | 0xF4 => oi!(2, 4, Normal, ZpX, "DOP"),
        0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC => oi!(3, 4, Normal, AbsX, "TOP"),
        0x80 | 0x82 | 0x89 | 0xC2 | 0xE2 => oi!(2, 2, Normal, Imm, "DOP"),

        // ── Illegal but used ──
        // LAX
        0xA7 => oi!(2, 3, Normal, Zp, "LAX"),
        0xB7 => oi!(2, 4, Normal, ZpY, "LAX"),
        0xAF => oi!(3, 4, Normal, Abs, "LAX"),
        0xBF => oi!(3, 4, Normal, AbsY, "LAX"),
        0xA3 => oi!(2, 6, Normal, IzX, "LAX"),
        0xB3 => oi!(2, 5, Normal, IzY, "LAX"),

        // SAX
        0x87 => oi!(2, 3, Normal, Zp, "SAX"),
        0x97 => oi!(2, 4, Normal, ZpY, "SAX"),
        0x8F => oi!(3, 4, Normal, Abs, "SAX"),
        0x83 => oi!(2, 6, Normal, IzX, "SAX"),

        // DCP
        0xC7 => oi!(2, 5, Normal, Zp, "DCP"),
        0xD7 => oi!(2, 6, Normal, ZpX, "DCP"),
        0xCF => oi!(3, 6, Normal, Abs, "DCP"),
        0xDF => oi!(3, 7, Normal, AbsX, "DCP"),
        0xDB => oi!(3, 7, Normal, AbsY, "DCP"),
        0xC3 => oi!(2, 8, Normal, IzX, "DCP"),
        0xD3 => oi!(2, 8, Normal, IzY, "DCP"),

        // ISB
        0xE7 => oi!(2, 5, Normal, Zp, "ISB"),
        0xF7 => oi!(2, 6, Normal, ZpX, "ISB"),
        0xEF => oi!(3, 6, Normal, Abs, "ISB"),
        0xFF => oi!(3, 7, Normal, AbsX, "ISB"),
        0xFB => oi!(3, 7, Normal, AbsY, "ISB"),
        0xE3 => oi!(2, 8, Normal, IzX, "ISB"),
        0xF3 => oi!(2, 8, Normal, IzY, "ISB"),

        // SLO
        0x07 => oi!(2, 5, Normal, Zp, "SLO"),
        0x17 => oi!(2, 6, Normal, ZpX, "SLO"),
        0x0F => oi!(3, 6, Normal, Abs, "SLO"),
        0x1F => oi!(3, 7, Normal, AbsX, "SLO"),
        0x1B => oi!(3, 7, Normal, AbsY, "SLO"),
        0x03 => oi!(2, 8, Normal, IzX, "SLO"),
        0x13 => oi!(2, 8, Normal, IzY, "SLO"),

        // RLA
        0x27 => oi!(2, 5, Normal, Zp, "RLA"),
        0x37 => oi!(2, 6, Normal, ZpX, "RLA"),
        0x2F => oi!(3, 6, Normal, Abs, "RLA"),
        0x3F => oi!(3, 7, Normal, AbsX, "RLA"),
        0x3B => oi!(3, 7, Normal, AbsY, "RLA"),
        0x23 => oi!(2, 8, Normal, IzX, "RLA"),
        0x33 => oi!(2, 8, Normal, IzY, "RLA"),

        // SRE
        0x47 => oi!(2, 5, Normal, Zp, "SRE"),
        0x57 => oi!(2, 6, Normal, ZpX, "SRE"),
        0x4F => oi!(3, 6, Normal, Abs, "SRE"),
        0x5F => oi!(3, 7, Normal, AbsX, "SRE"),
        0x5B => oi!(3, 7, Normal, AbsY, "SRE"),
        0x43 => oi!(2, 8, Normal, IzX, "SRE"),
        0x53 => oi!(2, 8, Normal, IzY, "SRE"),

        // RRA
        0x67 => oi!(2, 5, Normal, Zp, "RRA"),
        0x77 => oi!(2, 6, Normal, ZpX, "RRA"),
        0x6F => oi!(3, 6, Normal, Abs, "RRA"),
        0x7F => oi!(3, 7, Normal, AbsX, "RRA"),
        0x7B => oi!(3, 7, Normal, AbsY, "RRA"),
        0x63 => oi!(2, 8, Normal, IzX, "RRA"),
        0x73 => oi!(2, 8, Normal, IzY, "RRA"),

        // EB = unofficial SBC imm
        0xEB => oi!(2, 2, Normal, Imm, "SBC"),

        // Everything else: 1-byte NOP (KIL, unknown)
        _ => oi!(1, 2, Normal, Impl, "KIL"),
    }
}

// ── ROM parsing ──

fn find_rom() -> String {
    let candidates = [
        env::var("CONTRA_ROM_PATH").unwrap_or_default(),
        "/workspace/sfk/contra-speedrun/contra.nes".to_string(),
        "../contra.nes".to_string(),
        "contra.nes".to_string(),
    ];
    for p in &candidates {
        if !p.is_empty() && Path::new(p).exists() {
            return p.clone();
        }
    }
    panic!("Cannot find contra.nes ROM. Set CONTRA_ROM_PATH env var.");
}

fn parse_ines(data: &[u8]) -> (Vec<Vec<u8>>, usize, u8) {
    assert!(data.len() >= 16 && data[0..4] == [b'N', b'E', b'S', 0x1A]);
    let prg_bank_count = data[4] as usize;
    let flags6 = data[6];
    let mirroring = if flags6 & 1 == 1 { 1 } else { 0 }; // 1=vertical
    let has_trainer = flags6 & 4 != 0;
    let prg_start = 16 + if has_trainer { 512 } else { 0 };

    let mut banks = Vec::new();
    for i in 0..prg_bank_count {
        let start = prg_start + i * PRG_BANK_SIZE;
        banks.push(data[start..start + PRG_BANK_SIZE].to_vec());
    }
    (banks, prg_bank_count, mirroring)
}

// ── Disassembly ──

struct Instruction {
    addr: u16,
    opcode: u8,
    operand: [u8; 2],
    info: &'static OpcodeInfo,
}

// We leak the OpcodeInfo to get 'static references (fine for build script)
fn get_info(op: u8) -> &'static OpcodeInfo {
    Box::leak(Box::new(opcode_info(op)))
}

struct Block {
    bank: usize, // 0-6 for switchable, 7 for fixed
    start: u16,
    instructions: Vec<Instruction>,
}

fn read_prg(banks: &[Vec<u8>], bank: usize, addr: u16) -> u8 {
    if addr >= 0xC000 {
        banks[banks.len() - 1][(addr - 0xC000) as usize]
    } else if addr >= 0x8000 {
        banks[bank][(addr - 0x8000) as usize]
    } else {
        0
    }
}

fn disassemble(banks: &[Vec<u8>]) -> Vec<Block> {
    let last = banks.len() - 1;
    let mut all_blocks = Vec::new();

    // Get entry points from vectors (in fixed bank)
    let reset = banks[last][0x3FFC] as u16 | ((banks[last][0x3FFD] as u16) << 8);
    let nmi   = banks[last][0x3FFA] as u16 | ((banks[last][0x3FFB] as u16) << 8);
    let irq   = banks[last][0x3FFE] as u16 | ((banks[last][0x3FFF] as u16) << 8);

    eprintln!("  Vectors: RESET=${:04X} NMI=${:04X} IRQ=${:04X}", reset, nmi, irq);

    // For the fixed bank, disassemble from all entry points
    let fixed_entries = vec![reset, nmi, irq];
    let fixed_blocks = disassemble_bank(banks, last, &fixed_entries);

    // Collect JSR/JMP targets that land in $8000-$BFFF (switchable bank)
    let mut switch_targets: BTreeSet<u16> = BTreeSet::new();
    for block in &fixed_blocks {
        for inst in &block.instructions {
            let info = opcode_info(inst.opcode);
            let target_addr = inst.operand[0] as u16 | ((inst.operand[1] as u16) << 8);
            if (info.kind == InstKind::Jsr || info.kind == InstKind::JmpAbs)
                && target_addr >= 0x8000 && target_addr < 0xC000
            {
                switch_targets.insert(target_addr);
            }
        }
    }

    all_blocks.extend(fixed_blocks);

    // For each switchable bank, disassemble from switch targets + $8000
    let switch_entries: Vec<u16> = switch_targets.into_iter().collect();
    for bank_idx in 0..last {
        let mut entries = switch_entries.clone();
        entries.push(0x8000); // common entry
        entries.sort();
        entries.dedup();
        let bank_blocks = disassemble_bank(banks, bank_idx, &entries);
        all_blocks.extend(bank_blocks);
    }

    // Also scan for additional reachable blocks in switchable banks
    // by looking at JSR/JMP targets within each bank
    let mut more_targets: BTreeMap<usize, BTreeSet<u16>> = BTreeMap::new();
    for block in &all_blocks {
        for inst in &block.instructions {
            let info = opcode_info(inst.opcode);
            let target_addr = inst.operand[0] as u16 | ((inst.operand[1] as u16) << 8);
            if (info.kind == InstKind::Jsr || info.kind == InstKind::JmpAbs)
                && target_addr >= 0x8000 && target_addr < 0xC000
            {
                for b in 0..last {
                    more_targets.entry(b).or_default().insert(target_addr);
                }
            }
        }
    }

    for (bank_idx, targets) in &more_targets {
        let entries: Vec<u16> = targets.iter().copied().collect();
        let bank_blocks = disassemble_bank(banks, *bank_idx, &entries);
        // Only add blocks we don't already have
        for block in bank_blocks {
            let key = (block.bank, block.start);
            if !all_blocks.iter().any(|b| (b.bank, b.start) == key) {
                all_blocks.push(block);
            }
        }
    }

    // Deduplicate: use (bank_tag, start) as key
    // For $C000+ addresses, bank_tag is always "fx" regardless of discovering bank
    let mut seen: BTreeSet<(String, u16)> = BTreeSet::new();
    all_blocks.retain(|block| {
        let tag = if block.start >= 0xC000 { "fx".to_string() } else { format!("b{}", block.bank) };
        seen.insert((tag, block.start))
    });

    eprintln!("  Disassembled {} unique blocks", all_blocks.len());
    all_blocks
}

fn disassemble_bank(banks: &[Vec<u8>], bank_idx: usize, entries: &[u16]) -> Vec<Block> {
    let last = banks.len() - 1;
    let mut blocks = Vec::new();
    let mut visited: BTreeSet<u16> = BTreeSet::new();
    let mut work: VecDeque<u16> = VecDeque::new();

    for &e in entries {
        if e >= 0x8000 {
            work.push_back(e);
        }
    }

    while let Some(start) = work.pop_front() {
        if visited.contains(&start) || start < 0x8000 {
            continue;
        }
        // For non-last banks, skip fixed bank addresses (handled separately)
        if bank_idx != last && start >= 0xC000 {
            continue;
        }

        let mut addr = start;
        let mut instructions = Vec::new();
        let mut block_addrs: BTreeSet<u16> = BTreeSet::new();

        loop {
            if addr < 0x8000 || visited.contains(&addr) || block_addrs.contains(&addr) {
                break;
            }

            let bank_for_read = if addr >= 0xC000 { last } else { bank_idx };
            let op = read_prg(banks, bank_for_read, addr);
            let info = get_info(op);

            let mut operand = [0u8; 2];
            if info.len >= 2 {
                operand[0] = read_prg(banks, bank_for_read, addr.wrapping_add(1));
            }
            if info.len >= 3 {
                operand[1] = read_prg(banks, bank_for_read, addr.wrapping_add(2));
            }

            block_addrs.insert(addr);
            let next_addr = addr.wrapping_add(info.len as u16);

            instructions.push(Instruction {
                addr,
                opcode: op,
                operand,
                info,
            });

            // Check if this instruction ends the block
            let ends_block = match info.kind {
                InstKind::Branch => true,
                InstKind::JmpAbs | InstKind::JmpInd => true,
                InstKind::Jsr => true,
                InstKind::Rts | InstKind::Rti | InstKind::Brk => true,
                InstKind::Normal => {
                    // End block on writes to $4014 (DMA) or $8000+ (bank switch)
                    let is_store = matches!(info.name, "STA" | "STX" | "STY" | "SAX");
                    if is_store && info.mode == AddrMode::Abs {
                        let target = operand[0] as u16 | ((operand[1] as u16) << 8);
                        target == 0x4014 || target >= 0x8000
                    } else {
                        false
                    }
                }
            };

            if ends_block {
                // Add successor addresses to work queue
                match info.kind {
                    InstKind::Branch => {
                        let offset = operand[0] as i8 as i16;
                        let target = next_addr.wrapping_add(offset as u16);
                        work.push_back(target);
                        work.push_back(next_addr);
                    }
                    InstKind::JmpAbs => {
                        let target = operand[0] as u16 | ((operand[1] as u16) << 8);
                        work.push_back(target);
                    }
                    InstKind::Jsr => {
                        let target = operand[0] as u16 | ((operand[1] as u16) << 8);
                        work.push_back(target);
                        work.push_back(next_addr);
                    }
                    InstKind::Normal => {
                        // Store that ended block (DMA/bank switch)
                        work.push_back(next_addr);
                    }
                    _ => {} // RTS/RTI/BRK/JmpInd — can't determine target statically
                }
                break;
            }

            addr = next_addr;
        }

        if !instructions.is_empty() {
            // Mark all addresses in this block as visited
            for a in &block_addrs {
                visited.insert(*a);
            }
            blocks.push(Block {
                bank: bank_idx,
                start,
                instructions,
            });
        }
    }

    blocks
}

// ── Code generation ──

fn gen_read_value(mode: AddrMode, operand: [u8; 2]) -> String {
    let op8 = operand[0];
    let op16 = operand[0] as u16 | ((operand[1] as u16) << 8);
    match mode {
        AddrMode::Imm => format!("0x{:02X}u8", op8),
        // ZP reads: direct RAM access (addr < 0x100, always in RAM)
        AddrMode::Zp => format!("unsafe {{ *bus.ram.get_unchecked(0x{:02X}usize) }}", op8),
        AddrMode::ZpX => format!("unsafe {{ *bus.ram.get_unchecked((0x{:02X}u8).wrapping_add(s.x) as usize) }}", op8),
        AddrMode::ZpY => format!("unsafe {{ *bus.ram.get_unchecked((0x{:02X}u8).wrapping_add(s.y) as usize) }}", op8),
        // Absolute reads: inline for known RAM addresses
        AddrMode::Abs => {
            if op16 < 0x0800 {
                format!("unsafe {{ *bus.ram.get_unchecked(0x{:04X}usize) }}", op16)
            } else {
                format!("bus.read(0x{:04X})", op16)
            }
        }
        AddrMode::AbsX => format!("bus.read(0x{:04X}u16.wrapping_add(s.x as u16))", op16),
        AddrMode::AbsY => format!("bus.read(0x{:04X}u16.wrapping_add(s.y as u16))", op16),
        AddrMode::IzX => format!(
            "{{ let ptr = (0x{:02X}u8).wrapping_add(s.x); \
            let lo = unsafe {{ *bus.ram.get_unchecked(ptr as usize) }} as u16; \
            let hi = unsafe {{ *bus.ram.get_unchecked(ptr.wrapping_add(1) as usize) }} as u16; \
            bus.read(lo | (hi << 8)) }}", op8),
        AddrMode::IzY => format!(
            "{{ let lo = unsafe {{ *bus.ram.get_unchecked(0x{:02X}usize) }} as u16; \
            let hi = unsafe {{ *bus.ram.get_unchecked(0x{:02X}usize) }} as u16; \
            bus.read((lo | (hi << 8)).wrapping_add(s.y as u16)) }}",
            op8, (op8 as u8).wrapping_add(1)),
        _ => "0".to_string(),
    }
}

// Returns either a write address string (for use with bus.write) or "" for direct writes
fn gen_write_addr(mode: AddrMode, operand: [u8; 2]) -> String {
    let op8 = operand[0];
    let op16 = operand[0] as u16 | ((operand[1] as u16) << 8);
    match mode {
        AddrMode::Zp => format!("0x{:04X}u16", op8 as u16),
        AddrMode::ZpX => format!("(0x{:02X}u8).wrapping_add(s.x) as u16", op8),
        AddrMode::ZpY => format!("(0x{:02X}u8).wrapping_add(s.y) as u16", op8),
        AddrMode::Abs => format!("0x{:04X}u16", op16),
        AddrMode::AbsX => format!("0x{:04X}u16.wrapping_add(s.x as u16)", op16),
        AddrMode::AbsY => format!("0x{:04X}u16.wrapping_add(s.y as u16)", op16),
        AddrMode::IzX => format!(
            "{{ let ptr = (0x{:02X}u8).wrapping_add(s.x); \
            let lo = unsafe {{ *bus.ram.get_unchecked(ptr as usize) }} as u16; \
            let hi = unsafe {{ *bus.ram.get_unchecked(ptr.wrapping_add(1) as usize) }} as u16; \
            lo | (hi << 8) }}", op8),
        AddrMode::IzY => format!(
            "{{ let lo = unsafe {{ *bus.ram.get_unchecked(0x{:02X}usize) }} as u16; \
            let hi = unsafe {{ *bus.ram.get_unchecked(0x{:02X}usize) }} as u16; \
            (lo | (hi << 8)).wrapping_add(s.y as u16) }}",
            op8, (op8 as u8).wrapping_add(1)),
        _ => "0u16".to_string(),
    }
}

/// Generate store instruction with direct RAM access for ZP writes
fn gen_store(mode: AddrMode, operand: [u8; 2], val_expr: &str, cycles: u8) -> String {
    let op8 = operand[0];
    let op16 = operand[0] as u16 | ((operand[1] as u16) << 8);
    match mode {
        // ZP stores: direct RAM access
        AddrMode::Zp => format!(
            "unsafe {{ *bus.ram.get_unchecked_mut(0x{:02X}usize) = {}; }} s.cycles += {};",
            op8, val_expr, cycles),
        AddrMode::ZpX => format!(
            "unsafe {{ *bus.ram.get_unchecked_mut((0x{:02X}u8).wrapping_add(s.x) as usize) = {}; }} s.cycles += {};",
            op8, val_expr, cycles),
        AddrMode::ZpY => format!(
            "unsafe {{ *bus.ram.get_unchecked_mut((0x{:02X}u8).wrapping_add(s.y) as usize) = {}; }} s.cycles += {};",
            op8, val_expr, cycles),
        // Absolute stores: direct RAM for known RAM addresses
        AddrMode::Abs if op16 < 0x0800 => format!(
            "unsafe {{ *bus.ram.get_unchecked_mut(0x{:04X}usize) = {}; }} s.cycles += {};",
            op16, val_expr, cycles),
        _ => {
            let wa = gen_write_addr(mode, operand);
            format!("bus.write({}, {}); s.cycles += {};", wa, val_expr, cycles)
        }
    }
}

/// Generate read-modify-write instruction with direct RAM for ZP/known-RAM addresses
fn gen_rmw(mode: AddrMode, operand: [u8; 2], body: &str, cycles: u8) -> String {
    let op8 = operand[0];
    let op16 = operand[0] as u16 | ((operand[1] as u16) << 8);
    match mode {
        AddrMode::Zp => format!(
            "{{ let p = unsafe {{ bus.ram.get_unchecked_mut(0x{:02X}usize) }}; let mut v = *p; {} *p = v; s.cycles += {}; }}",
            op8, body, cycles),
        AddrMode::ZpX => format!(
            "{{ let p = unsafe {{ bus.ram.get_unchecked_mut((0x{:02X}u8).wrapping_add(s.x) as usize) }}; let mut v = *p; {} *p = v; s.cycles += {}; }}",
            op8, body, cycles),
        AddrMode::Abs if op16 < 0x0800 => format!(
            "{{ let p = unsafe {{ bus.ram.get_unchecked_mut(0x{:04X}usize) }}; let mut v = *p; {} *p = v; s.cycles += {}; }}",
            op16, body, cycles),
        _ => {
            let wa = gen_write_addr(mode, operand);
            format!("{{ let addr = {}; let mut v = bus.read(addr); {} bus.write(addr, v); s.cycles += {}; }}", wa, body, cycles)
        }
    }
}

fn gen_instruction(inst: &Instruction) -> String {
    let info = opcode_info(inst.opcode);
    let mode = info.mode;
    let operand = inst.operand;
    let cycles = info.cycles;
    let op8 = operand[0];
    let op16 = operand[0] as u16 | ((operand[1] as u16) << 8);

    match info.name {
        // ── Load ──
        "LDA" => {
            if mode == AddrMode::Imm {
                format!("s.a = 0x{:02X}; s.z = s.a == 0; s.n = s.a & 0x80 != 0; s.cycles += {};",
                    op8, cycles)
            } else {
                let rv = gen_read_value(mode, operand);
                format!("s.a = {}; s.z = s.a == 0; s.n = s.a & 0x80 != 0; s.cycles += {};",
                    rv, cycles)
            }
        }
        "LDX" => {
            if mode == AddrMode::Imm {
                format!("s.x = 0x{:02X}; s.z = s.x == 0; s.n = s.x & 0x80 != 0; s.cycles += {};",
                    op8, cycles)
            } else {
                let rv = gen_read_value(mode, operand);
                format!("s.x = {}; s.z = s.x == 0; s.n = s.x & 0x80 != 0; s.cycles += {};",
                    rv, cycles)
            }
        }
        "LDY" => {
            if mode == AddrMode::Imm {
                format!("s.y = 0x{:02X}; s.z = s.y == 0; s.n = s.y & 0x80 != 0; s.cycles += {};",
                    op8, cycles)
            } else {
                let rv = gen_read_value(mode, operand);
                format!("s.y = {}; s.z = s.y == 0; s.n = s.y & 0x80 != 0; s.cycles += {};",
                    rv, cycles)
            }
        }
        "LAX" => {
            let rv = gen_read_value(mode, operand);
            format!("s.a = {}; s.x = s.a; s.z = s.a == 0; s.n = s.a & 0x80 != 0; s.cycles += {};",
                rv, cycles)
        }

        // ── Store ──
        "STA" => gen_store(mode, operand, "s.a", cycles),
        "STX" => gen_store(mode, operand, "s.x", cycles),
        "STY" => gen_store(mode, operand, "s.y", cycles),
        "SAX" => gen_store(mode, operand, "s.a & s.x", cycles),

        // ── ALU ──
        "ADC" => {
            let rv = gen_read_value(mode, operand);
            format!("{{ let val = {}; s.adc(val); s.cycles += {}; }}", rv, cycles)
        }
        "SBC" => {
            let rv = gen_read_value(mode, operand);
            format!("{{ let val = {}; s.sbc(val); s.cycles += {}; }}", rv, cycles)
        }
        "CMP" => {
            let rv = gen_read_value(mode, operand);
            format!("{{ let val = {}; s.cmp_reg(s.a, val); s.cycles += {}; }}", rv, cycles)
        }
        "CPX" => {
            let rv = gen_read_value(mode, operand);
            format!("{{ let val = {}; s.cmp_reg(s.x, val); s.cycles += {}; }}", rv, cycles)
        }
        "CPY" => {
            let rv = gen_read_value(mode, operand);
            format!("{{ let val = {}; s.cmp_reg(s.y, val); s.cycles += {}; }}", rv, cycles)
        }
        "AND" => {
            let rv = gen_read_value(mode, operand);
            format!("s.a &= {}; s.z = s.a == 0; s.n = s.a & 0x80 != 0; s.cycles += {};",
                rv, cycles)
        }
        "ORA" => {
            let rv = gen_read_value(mode, operand);
            format!("s.a |= {}; s.z = s.a == 0; s.n = s.a & 0x80 != 0; s.cycles += {};",
                rv, cycles)
        }
        "EOR" => {
            let rv = gen_read_value(mode, operand);
            format!("s.a ^= {}; s.z = s.a == 0; s.n = s.a & 0x80 != 0; s.cycles += {};",
                rv, cycles)
        }
        "BIT" => {
            let rv = gen_read_value(mode, operand);
            format!("{{ let val = {}; s.z = s.a & val == 0; s.v = val & 0x40 != 0; s.n = val & 0x80 != 0; s.cycles += {}; }}",
                rv, cycles)
        }

        // ── Shifts (accumulator) ──
        "ASL" if mode == AddrMode::Acc => {
            "s.c = s.a & 0x80 != 0; s.a <<= 1; s.z = s.a == 0; s.n = s.a & 0x80 != 0; s.cycles += 2;".to_string()
        }
        "LSR" if mode == AddrMode::Acc => {
            "s.c = s.a & 1 != 0; s.a >>= 1; s.z = s.a == 0; s.n = false; s.cycles += 2;".to_string()
        }
        "ROL" if mode == AddrMode::Acc => {
            "{ let oc = s.c as u8; s.c = s.a & 0x80 != 0; s.a = (s.a << 1) | oc; s.z = s.a == 0; s.n = s.a & 0x80 != 0; s.cycles += 2; }".to_string()
        }
        "ROR" if mode == AddrMode::Acc => {
            "{ let oc = s.c as u8; s.c = s.a & 1 != 0; s.a = (s.a >> 1) | (oc << 7); s.z = s.a == 0; s.n = s.a & 0x80 != 0; s.cycles += 2; }".to_string()
        }

        // ── Shifts (memory) ──
        "ASL" => gen_rmw(mode, operand, "s.c = v & 0x80 != 0; v <<= 1; s.z = v == 0; s.n = v & 0x80 != 0;", cycles),
        "LSR" => gen_rmw(mode, operand, "s.c = v & 1 != 0; v >>= 1; s.z = v == 0; s.n = false;", cycles),
        "ROL" => gen_rmw(mode, operand, "let oc = s.c as u8; s.c = v & 0x80 != 0; v = (v << 1) | oc; s.z = v == 0; s.n = v & 0x80 != 0;", cycles),
        "ROR" => gen_rmw(mode, operand, "let oc = s.c as u8; s.c = v & 1 != 0; v = (v >> 1) | (oc << 7); s.z = v == 0; s.n = v & 0x80 != 0;", cycles),

        // ── Inc/Dec ──
        "INC" => gen_rmw(mode, operand, "v = v.wrapping_add(1); s.z = v == 0; s.n = v & 0x80 != 0;", cycles),
        "DEC" => gen_rmw(mode, operand, "v = v.wrapping_sub(1); s.z = v == 0; s.n = v & 0x80 != 0;", cycles),
        "INX" => "s.x = s.x.wrapping_add(1); s.z = s.x == 0; s.n = s.x & 0x80 != 0; s.cycles += 2;".to_string(),
        "INY" => "s.y = s.y.wrapping_add(1); s.z = s.y == 0; s.n = s.y & 0x80 != 0; s.cycles += 2;".to_string(),
        "DEX" => "s.x = s.x.wrapping_sub(1); s.z = s.x == 0; s.n = s.x & 0x80 != 0; s.cycles += 2;".to_string(),
        "DEY" => "s.y = s.y.wrapping_sub(1); s.z = s.y == 0; s.n = s.y & 0x80 != 0; s.cycles += 2;".to_string(),

        // ── Illegal RMW ──
        "DCP" => gen_rmw(mode, operand, "v = v.wrapping_sub(1); s.cmp_reg(s.a, v);", cycles),
        "ISB" => gen_rmw(mode, operand, "v = v.wrapping_add(1); s.sbc(v);", cycles),
        "SLO" => gen_rmw(mode, operand, "s.c = v & 0x80 != 0; v <<= 1; s.a |= v; s.z = s.a == 0; s.n = s.a & 0x80 != 0;", cycles),
        "RLA" => gen_rmw(mode, operand, "let oc = s.c as u8; s.c = v & 0x80 != 0; v = (v << 1) | oc; s.a &= v; s.z = s.a == 0; s.n = s.a & 0x80 != 0;", cycles),
        "SRE" => gen_rmw(mode, operand, "s.c = v & 1 != 0; v >>= 1; s.a ^= v; s.z = s.a == 0; s.n = s.a & 0x80 != 0;", cycles),
        "RRA" => gen_rmw(mode, operand, "let oc = s.c as u8; s.c = v & 1 != 0; v = (v >> 1) | (oc << 7); s.adc(v);", cycles),

        // ── Branches ──
        "BPL" | "BMI" | "BVC" | "BVS" | "BCC" | "BCS" | "BNE" | "BEQ" => {
            let offset = op8 as i8 as i16;
            let next_pc = inst.addr.wrapping_add(2);
            let target = next_pc.wrapping_add(offset as u16);
            let cond = match info.name {
                "BPL" => "!s.n", "BMI" => "s.n",
                "BVC" => "!s.v", "BVS" => "s.v",
                "BCC" => "!s.c", "BCS" => "s.c",
                "BNE" => "!s.z", "BEQ" => "s.z",
                _ => unreachable!(),
            };
            format!(
                "s.cycles += 2; if {} {{ s.cycles += 1; if 0x{:04X} & 0xFF00 != 0x{:04X} & 0xFF00 {{ s.cycles += 1; }} s.pc = 0x{:04X}; }} else {{ s.pc = 0x{:04X}; }}",
                cond, next_pc, target, target, next_pc
            )
        }

        // ── Jumps ──
        "JMP" if info.kind == InstKind::JmpAbs => {
            format!("s.pc = 0x{:04X}; s.cycles += 3;", op16)
        }
        "JMP" if info.kind == InstKind::JmpInd => {
            format!(
                "{{ let lo = bus.read(0x{:04X}) as u16; let hi_addr = (0x{:04X}u16 & 0xFF00) | ((0x{:04X}u16 + 1) & 0x00FF); let hi = bus.read(hi_addr) as u16; s.pc = lo | (hi << 8); s.cycles += 5; }}",
                op16, op16, op16
            )
        }
        "JSR" => {
            let return_addr = inst.addr.wrapping_add(2); // address of last byte of JSR instruction
            format!(
                "s.push16(bus, 0x{:04X}); s.pc = 0x{:04X}; s.cycles += 6;",
                return_addr, op16
            )
        }
        "RTS" => "s.pc = s.pull16(bus).wrapping_add(1); s.cycles += 6;".to_string(),
        "RTI" => "{ let st = s.pull(bus); s.set_status(st); s.pc = s.pull16(bus); s.cycles += 6; }".to_string(),

        // ── Stack/Transfer ──
        "TAX" => "s.x = s.a; s.z = s.x == 0; s.n = s.x & 0x80 != 0; s.cycles += 2;".to_string(),
        "TXA" => "s.a = s.x; s.z = s.a == 0; s.n = s.a & 0x80 != 0; s.cycles += 2;".to_string(),
        "TAY" => "s.y = s.a; s.z = s.y == 0; s.n = s.y & 0x80 != 0; s.cycles += 2;".to_string(),
        "TYA" => "s.a = s.y; s.z = s.a == 0; s.n = s.a & 0x80 != 0; s.cycles += 2;".to_string(),
        "TXS" => "s.sp = s.x; s.cycles += 2;".to_string(),
        "TSX" => "s.x = s.sp; s.z = s.x == 0; s.n = s.x & 0x80 != 0; s.cycles += 2;".to_string(),
        "PHA" => "s.push(bus, s.a); s.cycles += 3;".to_string(),
        "PLA" => "s.a = s.pull(bus); s.z = s.a == 0; s.n = s.a & 0x80 != 0; s.cycles += 4;".to_string(),
        "PHP" => "s.push(bus, s.status() | 0x10); s.cycles += 3;".to_string(),
        "PLP" => "{ let st = s.pull(bus); s.set_status(st); s.cycles += 4; }".to_string(),

        // ── Flags ──
        "CLC" => "s.c = false; s.cycles += 2;".to_string(),
        "SEC" => "s.c = true; s.cycles += 2;".to_string(),
        "CLI" => "s.i = false; s.cycles += 2;".to_string(),
        "SEI" => "s.i = true; s.cycles += 2;".to_string(),
        "CLV" => "s.v = false; s.cycles += 2;".to_string(),
        "CLD" => "s.d = false; s.cycles += 2;".to_string(),
        "SED" => "s.d = true; s.cycles += 2;".to_string(),

        // ── NOP / illegal NOPs ──
        "NOP" | "DOP" | "TOP" | "KIL" => {
            format!("s.cycles += {};", cycles)
        }

        // ── BRK ──
        "BRK" => {
            let next = inst.addr.wrapping_add(2);
            format!(
                "s.push16(bus, 0x{:04X}); s.push(bus, s.status() | 0x10); s.i = true; s.pc = bus.read(0xFFFE) as u16 | ((bus.read(0xFFFF) as u16) << 8); s.cycles += 7;",
                next
            )
        }

        _ => format!("s.cycles += {};", cycles),
    }
}

fn generate_code(out: &mut fs::File, blocks: &[Block], banks: &[Vec<u8>]) {
    let _last = banks.len() - 1;

    writeln!(out, "// Auto-generated by build.rs — do not edit").unwrap();
    writeln!(out, "// {} compiled blocks from {} PRG banks", blocks.len(), banks.len()).unwrap();
    writeln!(out, "// Types CpuState and Bus are imported by the including module.").unwrap();
    writeln!(out, "").unwrap();

    // Emit embedded PRG ROM data
    let total_prg: usize = banks.iter().map(|b| b.len()).sum();
    writeln!(out, "/// Embedded PRG ROM ({} banks, {} bytes)", banks.len(), total_prg).unwrap();
    writeln!(out, "pub const EMBEDDED_PRG: [u8; {}] = [", total_prg).unwrap();
    for bank in banks {
        for chunk in bank.chunks(32) {
            write!(out, "    ").unwrap();
            for b in chunk {
                write!(out, "0x{:02X},", b).unwrap();
            }
            writeln!(out).unwrap();
        }
    }
    writeln!(out, "];").unwrap();
    writeln!(out, "pub const EMBEDDED_PRG_BANKS: usize = {};", banks.len()).unwrap();
    writeln!(out, "").unwrap();

    // Generate block functions
    for block in blocks {
        let bank_tag = if block.start >= 0xC000 { "fx".to_string() } else { format!("b{}", block.bank) };
        let fn_name = format!("block_{}_0x{:04X}", bank_tag, block.start);

        writeln!(out, "#[inline(never)]").unwrap();
        writeln!(out, "fn {}(s: &mut CpuState, bus: &mut Bus) {{", fn_name).unwrap();

        let last_idx = block.instructions.len() - 1;
        for (i, inst) in block.instructions.iter().enumerate() {
            let code = gen_instruction(inst);
            let info = opcode_info(inst.opcode);
            writeln!(out, "    // ${:04X}: {} {:02X}", inst.addr, info.name, inst.opcode).unwrap();
            writeln!(out, "    {}", code).unwrap();

            // For non-terminal normal instructions that are the last in a block
            // (e.g., store to $4014 or $8000+), set PC to next instruction
            if i == last_idx && info.kind == InstKind::Normal {
                let next = inst.addr.wrapping_add(info.len as u16);
                writeln!(out, "    s.pc = 0x{:04X};", next).unwrap();
            }
        }

        writeln!(out, "}}").unwrap();
        writeln!(out, "").unwrap();
    }

    // Generate dispatch function — single match on (key, pc)
    // key = 255 for fixed bank, bank index for switchable
    writeln!(out, "/// Dispatch to compiled block. Returns true if block was found.").unwrap();
    writeln!(out, "pub fn dispatch(bank: usize, pc: u16, s: &mut CpuState, bus: &mut Bus) -> bool {{").unwrap();
    writeln!(out, "    let key = if pc >= 0xC000 {{ 255u16 }} else {{ bank as u16 }};").unwrap();
    writeln!(out, "    match (key, pc) {{").unwrap();

    for block in blocks {
        let bank_key = if block.start >= 0xC000 { 255u16 } else { block.bank as u16 };
        let bank_tag = if block.start >= 0xC000 { "fx".to_string() } else { format!("b{}", block.bank) };
        let fn_name = format!("block_{}_0x{:04X}", bank_tag, block.start);
        writeln!(out, "        ({}, 0x{:04X}) => {}(s, bus),", bank_key, block.start, fn_name).unwrap();
    }

    writeln!(out, "        _ => return false,").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    true").unwrap();
    writeln!(out, "}}").unwrap();
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let rom_path = find_rom();
    println!("cargo:rerun-if-changed={}", rom_path);
    eprintln!("  ROM: {}", rom_path);

    let rom_data = fs::read(&rom_path).expect("Failed to read ROM");
    let (banks, bank_count, _mirroring) = parse_ines(&rom_data);
    eprintln!("  PRG banks: {} ({}KB)", bank_count, bank_count * 16);

    let blocks = disassemble(&banks);

    let out_dir = env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir).join("compiled_blocks.rs");
    let mut out = fs::File::create(&out_path).unwrap();

    generate_code(&mut out, &blocks, &banks);

    eprintln!("  Generated: {} ({} blocks)", out_path.display(), blocks.len());
}
