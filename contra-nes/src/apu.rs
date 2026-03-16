// NES APU (2A03) — pulse, triangle, noise, DMC
//
// Frame counter drives envelope/length/sweep at 240Hz.
// Audio output mixed to mono f32 samples at ~1.79MHz, downsampled to 44.1kHz.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub const SAMPLE_RATE: u32 = 44100;
const CPU_FREQ: f64 = 1_789_773.0;

// Length counter lookup table
static LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14,
    12, 16, 24, 18, 48, 20, 96, 22, 192, 24, 72, 26, 16, 28, 32, 30,
];

// Duty cycle sequences for pulse channels
static DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0], // 12.5%
    [0, 1, 1, 0, 0, 0, 0, 0], // 25%
    [0, 1, 1, 1, 1, 0, 0, 0], // 50%
    [1, 0, 0, 1, 1, 1, 1, 1], // 75% (inverted 25%)
];

// Triangle waveform
static TRIANGLE_TABLE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];

// Noise period table (NTSC)
static NOISE_TABLE: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

// DMC rate table (NTSC)
static DMC_TABLE: [u16; 16] = [
    428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54,
];

// ── Envelope ──

#[derive(Clone)]
struct Envelope {
    start: bool,
    loop_flag: bool,
    constant: bool,
    volume: u8,    // V parameter (constant volume or envelope period)
    decay: u8,     // current decay level
    divider: u8,
}

impl Envelope {
    fn new() -> Self {
        Envelope { start: false, loop_flag: false, constant: false, volume: 0, decay: 0, divider: 0 }
    }

    fn clock(&mut self) {
        if self.start {
            self.start = false;
            self.decay = 15;
            self.divider = self.volume;
        } else if self.divider == 0 {
            self.divider = self.volume;
            if self.decay > 0 {
                self.decay -= 1;
            } else if self.loop_flag {
                self.decay = 15;
            }
        } else {
            self.divider -= 1;
        }
    }

    fn output(&self) -> u8 {
        if self.constant { self.volume } else { self.decay }
    }
}

// ── Sweep ──

#[derive(Clone)]
struct Sweep {
    enabled: bool,
    period: u8,
    negate: bool,
    shift: u8,
    reload: bool,
    divider: u8,
    is_pulse1: bool,
}

impl Sweep {
    fn new(is_pulse1: bool) -> Self {
        Sweep { enabled: false, period: 0, negate: false, shift: 0, reload: false, divider: 0, is_pulse1 }
    }

    fn target_period(&self, current: u16) -> u16 {
        let delta = current >> self.shift;
        if self.negate {
            if self.is_pulse1 {
                current.wrapping_sub(delta).wrapping_sub(1) // pulse 1: ones' complement
            } else {
                current.wrapping_sub(delta) // pulse 2: two's complement
            }
        } else {
            current + delta
        }
    }

    fn clock(&mut self, timer: &mut u16) {
        let target = self.target_period(*timer);
        if self.divider == 0 && self.enabled && self.shift > 0 && *timer >= 8 && target <= 0x7FF {
            *timer = target;
        }
        if self.divider == 0 || self.reload {
            self.divider = self.period;
            self.reload = false;
        } else {
            self.divider -= 1;
        }
    }

    fn muting(&self, timer: u16) -> bool {
        timer < 8 || self.target_period(timer) > 0x7FF
    }
}

// ── Pulse channel ──

#[derive(Clone)]
struct Pulse {
    enabled: bool,
    duty: u8,
    length_halt: bool,
    length: u8,
    envelope: Envelope,
    sweep: Sweep,
    timer: u16,
    timer_val: u16,
    seq_pos: u8,
}

impl Pulse {
    fn new(is_pulse1: bool) -> Self {
        Pulse {
            enabled: false, duty: 0, length_halt: false, length: 0,
            envelope: Envelope::new(), sweep: Sweep::new(is_pulse1),
            timer: 0, timer_val: 0, seq_pos: 0,
        }
    }

    fn write_reg(&mut self, addr: u8, val: u8) {
        match addr {
            0 => {
                self.duty = (val >> 6) & 3;
                self.length_halt = val & 0x20 != 0;
                self.envelope.loop_flag = val & 0x20 != 0;
                self.envelope.constant = val & 0x10 != 0;
                self.envelope.volume = val & 0x0F;
            }
            1 => {
                self.sweep.enabled = val & 0x80 != 0;
                self.sweep.period = (val >> 4) & 7;
                self.sweep.negate = val & 0x08 != 0;
                self.sweep.shift = val & 7;
                self.sweep.reload = true;
            }
            2 => {
                self.timer = (self.timer & 0xFF00) | val as u16;
            }
            3 => {
                self.timer = (self.timer & 0x00FF) | ((val as u16 & 7) << 8);
                if self.enabled {
                    self.length = LENGTH_TABLE[(val >> 3) as usize];
                }
                self.seq_pos = 0;
                self.envelope.start = true;
            }
            _ => {}
        }
    }

    fn clock_timer(&mut self) {
        if self.timer_val == 0 {
            self.timer_val = self.timer;
            self.seq_pos = (self.seq_pos + 1) & 7;
        } else {
            self.timer_val -= 1;
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled || self.length == 0 || self.sweep.muting(self.timer)
            || DUTY_TABLE[self.duty as usize][self.seq_pos as usize] == 0
        {
            0
        } else {
            self.envelope.output()
        }
    }
}

// ── Triangle channel ──

#[derive(Clone)]
struct Triangle {
    enabled: bool,
    control: bool, // length counter halt / linear counter control
    length: u8,
    linear_load: u8,
    linear_counter: u8,
    linear_reload: bool,
    timer: u16,
    timer_val: u16,
    seq_pos: u8,
}

impl Triangle {
    fn new() -> Self {
        Triangle {
            enabled: false, control: false, length: 0,
            linear_load: 0, linear_counter: 0, linear_reload: false,
            timer: 0, timer_val: 0, seq_pos: 0,
        }
    }

    fn write_reg(&mut self, addr: u8, val: u8) {
        match addr {
            0 => {
                self.control = val & 0x80 != 0;
                self.linear_load = val & 0x7F;
            }
            2 => {
                self.timer = (self.timer & 0xFF00) | val as u16;
            }
            3 => {
                self.timer = (self.timer & 0x00FF) | ((val as u16 & 7) << 8);
                if self.enabled {
                    self.length = LENGTH_TABLE[(val >> 3) as usize];
                }
                self.linear_reload = true;
            }
            _ => {}
        }
    }

    fn clock_timer(&mut self) {
        if self.timer_val == 0 {
            self.timer_val = self.timer;
            if self.length > 0 && self.linear_counter > 0 {
                self.seq_pos = (self.seq_pos + 1) & 31;
            }
        } else {
            self.timer_val -= 1;
        }
    }

    fn clock_linear(&mut self) {
        if self.linear_reload {
            self.linear_counter = self.linear_load;
        } else if self.linear_counter > 0 {
            self.linear_counter -= 1;
        }
        if !self.control {
            self.linear_reload = false;
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled || self.length == 0 || self.linear_counter == 0 || self.timer < 2 {
            0
        } else {
            TRIANGLE_TABLE[self.seq_pos as usize]
        }
    }
}

// ── Noise channel ──

#[derive(Clone)]
struct Noise {
    enabled: bool,
    length_halt: bool,
    length: u8,
    envelope: Envelope,
    mode: bool, // short mode (bit 6)
    period_idx: u8,
    timer_val: u16,
    shift: u16, // 15-bit LFSR
}

impl Noise {
    fn new() -> Self {
        Noise {
            enabled: false, length_halt: false, length: 0,
            envelope: Envelope::new(), mode: false, period_idx: 0,
            timer_val: 0, shift: 1,
        }
    }

    fn write_reg(&mut self, addr: u8, val: u8) {
        match addr {
            0 => {
                self.length_halt = val & 0x20 != 0;
                self.envelope.loop_flag = val & 0x20 != 0;
                self.envelope.constant = val & 0x10 != 0;
                self.envelope.volume = val & 0x0F;
            }
            2 => {
                self.mode = val & 0x80 != 0;
                self.period_idx = val & 0x0F;
            }
            3 => {
                if self.enabled {
                    self.length = LENGTH_TABLE[(val >> 3) as usize];
                }
                self.envelope.start = true;
            }
            _ => {}
        }
    }

    fn clock_timer(&mut self) {
        if self.timer_val == 0 {
            self.timer_val = NOISE_TABLE[self.period_idx as usize];
            let bit = if self.mode { 6 } else { 1 };
            let feedback = (self.shift & 1) ^ ((self.shift >> bit) & 1);
            self.shift = (self.shift >> 1) | (feedback << 14);
        } else {
            self.timer_val -= 1;
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled || self.length == 0 || self.shift & 1 != 0 {
            0
        } else {
            self.envelope.output()
        }
    }
}

// ── DMC channel ──

#[derive(Clone)]
struct Dmc {
    enabled: bool,
    irq_enabled: bool,
    loop_flag: bool,
    rate_idx: u8,
    timer_val: u16,
    output_level: u8,
    sample_addr: u16,
    sample_len: u16,
    current_addr: u16,
    bytes_remaining: u16,
    shift_register: u8,
    bits_remaining: u8,
    sample_buffer: u8,
    buffer_empty: bool,
    silence: bool,
}

impl Dmc {
    fn new() -> Self {
        Dmc {
            enabled: false, irq_enabled: false, loop_flag: false,
            rate_idx: 0, timer_val: 0, output_level: 0,
            sample_addr: 0xC000, sample_len: 1,
            current_addr: 0xC000, bytes_remaining: 0,
            shift_register: 0, bits_remaining: 0,
            sample_buffer: 0, buffer_empty: true, silence: true,
        }
    }

    fn write_reg(&mut self, addr: u8, val: u8) {
        match addr {
            0 => {
                self.irq_enabled = val & 0x80 != 0;
                self.loop_flag = val & 0x40 != 0;
                self.rate_idx = val & 0x0F;
            }
            1 => {
                self.output_level = val & 0x7F;
            }
            2 => {
                self.sample_addr = 0xC000 | ((val as u16) << 6);
            }
            3 => {
                self.sample_len = ((val as u16) << 4) | 1;
            }
            _ => {}
        }
    }

    fn restart(&mut self) {
        self.current_addr = self.sample_addr;
        self.bytes_remaining = self.sample_len;
    }

    // Returns true if a CPU read is needed (sample_addr to fetch)
    fn clock_timer(&mut self) -> Option<u16> {
        let mut fetch_addr = None;

        if self.timer_val == 0 {
            self.timer_val = DMC_TABLE[self.rate_idx as usize];

            if !self.silence {
                if self.shift_register & 1 != 0 {
                    if self.output_level <= 125 { self.output_level += 2; }
                } else {
                    if self.output_level >= 2 { self.output_level -= 2; }
                }
                self.shift_register >>= 1;
            }

            self.bits_remaining = self.bits_remaining.saturating_sub(1);
            if self.bits_remaining == 0 {
                self.bits_remaining = 8;
                if self.buffer_empty {
                    self.silence = true;
                } else {
                    self.silence = false;
                    self.shift_register = self.sample_buffer;
                    self.buffer_empty = true;
                }
            }

            // Request next sample byte if buffer is empty
            if self.buffer_empty && self.bytes_remaining > 0 {
                fetch_addr = Some(self.current_addr);
                self.current_addr = self.current_addr.wrapping_add(1) | 0x8000;
                self.bytes_remaining -= 1;
                if self.bytes_remaining == 0 && self.loop_flag {
                    self.restart();
                }
            }
        } else {
            self.timer_val -= 1;
        }

        fetch_addr
    }

    fn output(&self) -> u8 {
        self.output_level
    }
}

// ── Lock-free audio ring buffer ──
// Single-producer (emulator thread), single-consumer (cpal audio callback).
// Uses atomic indices so no mutex is needed.

pub struct AudioBuffer {
    buf: Vec<f32>,
    write_pos: AtomicUsize,
    read_pos: AtomicUsize,
    capacity: usize,
}

// Safety: buf is only written by one thread (producer at write_pos)
// and only read by one thread (consumer at read_pos). The atomics
// ensure proper ordering.
unsafe impl Send for AudioBuffer {}
unsafe impl Sync for AudioBuffer {}

impl AudioBuffer {
    pub fn new(capacity: usize) -> Self {
        AudioBuffer {
            buf: vec![0.0; capacity],
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
            capacity,
        }
    }

    /// Producer: write a sample (drops if full)
    pub fn write(&self, sample: f32) {
        let wp = self.write_pos.load(Ordering::Relaxed);
        let next = (wp + 1) % self.capacity;
        if next != self.read_pos.load(Ordering::Acquire) {
            // Safety: only the producer writes to buf[wp], and we checked it's not
            // overlapping with the consumer's read position
            let ptr = self.buf.as_ptr() as *mut f32;
            unsafe { ptr.add(wp).write(sample); }
            self.write_pos.store(next, Ordering::Release);
        }
    }

    /// Consumer: read a sample (returns 0 on underrun)
    pub fn read(&self) -> f32 {
        let rp = self.read_pos.load(Ordering::Relaxed);
        if rp == self.write_pos.load(Ordering::Acquire) {
            return 0.0;
        }
        let val = self.buf[rp];
        self.read_pos.store((rp + 1) % self.capacity, Ordering::Release);
        val
    }

    #[allow(dead_code)]
    pub fn available(&self) -> usize {
        let wp = self.write_pos.load(Ordering::Relaxed);
        let rp = self.read_pos.load(Ordering::Relaxed);
        if wp >= rp { wp - rp } else { self.capacity - rp + wp }
    }
}

// ── Main APU ──

pub struct Apu {
    pulse1: Pulse,
    pulse2: Pulse,
    triangle: Triangle,
    noise: Noise,
    dmc: Dmc,

    // Frame counter
    frame_mode: u8,     // 0 = 4-step, 1 = 5-step
    pub frame_irq: bool,
    frame_irq_inhibit: bool,
    frame_counter: u32,
    _frame_step: u8,

    // Sample output with low-pass filtering
    cycle_count: u64,
    sample_phase: f64,        // fractional phase accumulator for resampling
    cycles_per_sample: f64,
    filter_prev: f32,         // first-order low-pass state
    high_pass1: f32,          // first high-pass filter (removes DC offset ~90Hz)
    high_pass2: f32,          // second high-pass filter (~440Hz for crispness)

    pub audio_buf: Arc<AudioBuffer>,

    // DMC memory read callback result
    pub dmc_read_pending: Option<u16>,
}

impl Apu {
    pub fn new() -> Self {
        // ~3000 samples = ~68ms at 44.1kHz — enough for ~4 frames of audio,
        // small enough to keep latency tight
        let buf = Arc::new(AudioBuffer::new(4096));
        Apu {
            pulse1: Pulse::new(true),
            pulse2: Pulse::new(false),
            triangle: Triangle::new(),
            noise: Noise::new(),
            dmc: Dmc::new(),
            frame_mode: 0,
            frame_irq: false,
            frame_irq_inhibit: false,
            frame_counter: 0,
            _frame_step: 0,
            cycle_count: 0,
            sample_phase: 0.0,
            cycles_per_sample: CPU_FREQ / SAMPLE_RATE as f64,
            filter_prev: 0.0,
            high_pass1: 0.0,
            high_pass2: 0.0,
            audio_buf: buf,
            dmc_read_pending: None,
        }
    }

    pub fn audio_buffer(&self) -> Arc<AudioBuffer> {
        Arc::clone(&self.audio_buf)
    }

    pub fn write_register(&mut self, addr: u16, val: u8) {
        match addr {
            // Pulse 1: $4000-$4003
            0x4000 => self.pulse1.write_reg(0, val),
            0x4001 => self.pulse1.write_reg(1, val),
            0x4002 => self.pulse1.write_reg(2, val),
            0x4003 => self.pulse1.write_reg(3, val),
            // Pulse 2: $4004-$4007
            0x4004 => self.pulse2.write_reg(0, val),
            0x4005 => self.pulse2.write_reg(1, val),
            0x4006 => self.pulse2.write_reg(2, val),
            0x4007 => self.pulse2.write_reg(3, val),
            // Triangle: $4008, $400A, $400B
            0x4008 => self.triangle.write_reg(0, val),
            0x400A => self.triangle.write_reg(2, val),
            0x400B => self.triangle.write_reg(3, val),
            // Noise: $400C, $400E, $400F
            0x400C => self.noise.write_reg(0, val),
            0x400E => self.noise.write_reg(2, val),
            0x400F => self.noise.write_reg(3, val),
            // DMC: $4010-$4013
            0x4010 => self.dmc.write_reg(0, val),
            0x4011 => self.dmc.write_reg(1, val),
            0x4012 => self.dmc.write_reg(2, val),
            0x4013 => self.dmc.write_reg(3, val),
            // Status: $4015
            0x4015 => {
                self.pulse1.enabled = val & 1 != 0;
                self.pulse2.enabled = val & 2 != 0;
                self.triangle.enabled = val & 4 != 0;
                self.noise.enabled = val & 8 != 0;
                self.dmc.enabled = val & 0x10 != 0;

                if !self.pulse1.enabled { self.pulse1.length = 0; }
                if !self.pulse2.enabled { self.pulse2.length = 0; }
                if !self.triangle.enabled { self.triangle.length = 0; }
                if !self.noise.enabled { self.noise.length = 0; }

                if self.dmc.enabled {
                    if self.dmc.bytes_remaining == 0 { self.dmc.restart(); }
                } else {
                    self.dmc.bytes_remaining = 0;
                }
            }
            // Frame counter: $4017
            0x4017 => {
                self.frame_mode = (val >> 7) & 1;
                self.frame_irq_inhibit = val & 0x40 != 0;
                if self.frame_irq_inhibit { self.frame_irq = false; }
                self.frame_counter = 0;
                if self.frame_mode == 1 {
                    self.clock_quarter_frame();
                    self.clock_half_frame();
                }
            }
            _ => {}
        }
    }

    pub fn read_status(&mut self) -> u8 {
        let mut val = 0u8;
        if self.pulse1.length > 0 { val |= 1; }
        if self.pulse2.length > 0 { val |= 2; }
        if self.triangle.length > 0 { val |= 4; }
        if self.noise.length > 0 { val |= 8; }
        if self.dmc.bytes_remaining > 0 { val |= 0x10; }
        if self.frame_irq { val |= 0x40; }
        self.frame_irq = false;
        val
    }

    fn clock_quarter_frame(&mut self) {
        self.pulse1.envelope.clock();
        self.pulse2.envelope.clock();
        self.noise.envelope.clock();
        self.triangle.clock_linear();
    }

    fn clock_half_frame(&mut self) {
        // Length counters
        if !self.pulse1.length_halt && self.pulse1.length > 0 { self.pulse1.length -= 1; }
        if !self.pulse2.length_halt && self.pulse2.length > 0 { self.pulse2.length -= 1; }
        if !self.triangle.control && self.triangle.length > 0 { self.triangle.length -= 1; }
        if !self.noise.length_halt && self.noise.length > 0 { self.noise.length -= 1; }

        // Sweep
        self.pulse1.sweep.clock(&mut self.pulse1.timer);
        self.pulse2.sweep.clock(&mut self.pulse2.timer);
    }

    /// Clock the APU once per CPU cycle
    pub fn clock(&mut self) {
        self.cycle_count += 1;

        // Triangle clocks every CPU cycle
        self.triangle.clock_timer();

        // Pulse and noise clock every other CPU cycle
        if self.cycle_count & 1 == 0 {
            self.pulse1.clock_timer();
            self.pulse2.clock_timer();
            self.noise.clock_timer();
        }

        // DMC
        if let Some(addr) = self.dmc.clock_timer() {
            self.dmc_read_pending = Some(addr);
        }
        // Feed DMC sample byte if available
        if self.dmc.buffer_empty && self.dmc_read_pending.is_none() && self.dmc.bytes_remaining > 0 {
            // Will be handled by bus
        }

        // Frame counter (runs at CPU clock / 7457.5 ≈ 240Hz)
        self.frame_counter += 1;

        match self.frame_mode {
            0 => { // 4-step
                match self.frame_counter {
                    3729 => self.clock_quarter_frame(),
                    7457 => { self.clock_quarter_frame(); self.clock_half_frame(); }
                    11186 => self.clock_quarter_frame(),
                    14915 => {
                        self.clock_quarter_frame();
                        self.clock_half_frame();
                        if !self.frame_irq_inhibit { self.frame_irq = true; }
                        self.frame_counter = 0;
                    }
                    _ => {}
                }
            }
            1 => { // 5-step
                match self.frame_counter {
                    3729 => self.clock_quarter_frame(),
                    7457 => { self.clock_quarter_frame(); self.clock_half_frame(); }
                    11186 => self.clock_quarter_frame(),
                    18641 => {
                        self.clock_quarter_frame();
                        self.clock_half_frame();
                        self.frame_counter = 0;
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        // Downsample: emit one sample per output period
        // Mix + filter only at the output rate (~44.1kHz) instead of every CPU cycle
        // (~1.79MHz). This is 40x less work with negligible quality difference since
        // the NES channels are band-limited by their timer periods.
        self.sample_phase += 1.0;
        if self.sample_phase >= self.cycles_per_sample {
            self.sample_phase -= self.cycles_per_sample;

            let raw = self.mix();

            // Low-pass filter (smooths the stepped waveform)
            self.filter_prev += 0.5 * (raw - self.filter_prev);

            let inp = self.filter_prev;

            // High-pass #1: remove DC offset (~14Hz tracking)
            self.high_pass1 += (inp - self.high_pass1) * 0.002;
            let out = inp - self.high_pass1;

            // High-pass #2: steeper DC rolloff
            self.high_pass2 += (out - self.high_pass2) * 0.002;
            let out = out - self.high_pass2;

            // Scale and clamp
            let final_sample = (out * 1.2).clamp(-1.0, 1.0);
            self.audio_buf.write(final_sample);
        }
    }

    /// NES mixer — nonlinear mixing (outputs 0.0 to ~1.0)
    fn mix(&self) -> f32 {
        let p1 = self.pulse1.output() as f32;
        let p2 = self.pulse2.output() as f32;
        let t = self.triangle.output() as f32;
        let n = self.noise.output() as f32;
        let d = self.dmc.output() as f32;

        // Pulse output (0.0 to ~0.256)
        let pulse_out = if p1 + p2 > 0.0 {
            95.88 / (8128.0 / (p1 + p2) + 100.0)
        } else {
            0.0
        };

        // TND output (0.0 to ~0.741)
        let tnd_out = if t + n + d > 0.0 {
            159.79 / (1.0 / (t / 8227.0 + n / 12241.0 + d / 22638.0) + 100.0)
        } else {
            0.0
        };

        pulse_out + tnd_out // 0.0 to ~1.0 (DC removed by high-pass filters)
    }

    /// Feed DMC a byte read from memory
    pub fn dmc_fill_buffer(&mut self, byte: u8) {
        self.dmc.sample_buffer = byte;
        self.dmc.buffer_empty = false;
        self.dmc_read_pending = None;
    }
}
