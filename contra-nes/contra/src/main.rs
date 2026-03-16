#![allow(warnings)]
// contra-compiled: Statically recompiled Contra NES
//
// The 6502 game code is compiled to native Rust at build time.
// PPU/APU are identical to the interpreter for pixel-perfect accuracy.

mod apu;
mod bus;
mod cartridge;
mod cpu_state;
mod interpreter;
mod nes;
mod ppu;
mod renderer;
mod sprite;
mod tile_map;

use std::env;
use std::io::Write;
use std::time::Instant;

use crate::nes::Nes;

const SCREEN_W: usize = 256;
const SCREEN_H: usize = 240;

const BTN_A: u8 = 0;
const BTN_B: u8 = 1;
const BTN_SELECT: u8 = 2;
const BTN_START: u8 = 3;
const BTN_UP: u8 = 4;
const BTN_DOWN: u8 = 5;
const BTN_LEFT: u8 = 6;
const BTN_RIGHT: u8 = 7;

struct InputEvent { frame: u32, button: u8, pressed: bool }

fn build_autoplay_sequence() -> Vec<InputEvent> {
    let mut seq = Vec::new();
    let mut f = 250u32;
    let mut tap = |seq: &mut Vec<InputEvent>, btn: u8, dur: u32| {
        seq.push(InputEvent { frame: f, button: btn, pressed: true });
        seq.push(InputEvent { frame: f + dur, button: btn, pressed: false });
        f += dur + 6;
    };
    tap(&mut seq, BTN_UP, 4); tap(&mut seq, BTN_UP, 4);
    tap(&mut seq, BTN_DOWN, 4); tap(&mut seq, BTN_DOWN, 4);
    tap(&mut seq, BTN_LEFT, 4); tap(&mut seq, BTN_RIGHT, 4);
    tap(&mut seq, BTN_LEFT, 4); tap(&mut seq, BTN_RIGHT, 4);
    tap(&mut seq, BTN_B, 4); tap(&mut seq, BTN_A, 4);
    tap(&mut seq, BTN_START, 4);
    let gs = f + 200;
    seq.push(InputEvent { frame: gs, button: BTN_RIGHT, pressed: true });
    for i in 0..500 {
        let t = gs + 50 + i * 45;
        seq.push(InputEvent { frame: t, button: BTN_A, pressed: true });
        seq.push(InputEvent { frame: t + 8, button: BTN_A, pressed: false });
        seq.push(InputEvent { frame: t + 3, button: BTN_B, pressed: true });
        seq.push(InputEvent { frame: t + 7, button: BTN_B, pressed: false });
    }
    seq.sort_by_key(|e| e.frame);
    seq
}

fn write_bmp(path: &str, px: &[u32], w: u32, h: u32) {
    let stride = ((w * 3 + 3) / 4 * 4) as usize;
    let data_sz = stride * h as usize;
    let mut f = std::fs::File::create(path).unwrap();
    let mut hdr = [0u8; 54];
    hdr[0] = b'B'; hdr[1] = b'M';
    hdr[2..6].copy_from_slice(&((54 + data_sz) as u32).to_le_bytes());
    hdr[10..14].copy_from_slice(&54u32.to_le_bytes());
    hdr[14..18].copy_from_slice(&40u32.to_le_bytes());
    hdr[18..22].copy_from_slice(&w.to_le_bytes());
    hdr[22..26].copy_from_slice(&h.to_le_bytes());
    hdr[26..28].copy_from_slice(&1u16.to_le_bytes());
    hdr[28..30].copy_from_slice(&24u16.to_le_bytes());
    hdr[34..38].copy_from_slice(&(data_sz as u32).to_le_bytes());
    f.write_all(&hdr).unwrap();
    let mut row = vec![0u8; stride];
    for y in (0..h as usize).rev() {
        for x in 0..w as usize {
            let argb = px[y * w as usize + x];
            row[x*3] = argb as u8; row[x*3+1] = (argb >> 8) as u8; row[x*3+2] = (argb >> 16) as u8;
        }
        f.write_all(&row).unwrap();
    }
}

fn request_realtime_priority() {
    #[cfg(target_os = "linux")]
    {
        let param = libc::sched_param { sched_priority: 10 };
        let ret = unsafe { libc::sched_setscheduler(0, libc::SCHED_RR, &param) };
        if ret == 0 {
            eprintln!("  Thread: realtime priority (SCHED_RR)");
        }
    }
    #[cfg(target_os = "macos")]
    {
        eprintln!("  Thread: default priority");
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut mode = "window";
    let mut max_frames = u32::MAX;
    let mut scale = 4usize;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--window" | "-w" => mode = "window",
            "--benchmark" | "-b" => mode = "benchmark",
            "--headless" => mode = "headless",
            "--frames" | "-n" => { i += 1; max_frames = args[i].parse().unwrap_or(6000); }
            "--scale" | "-s" => { i += 1; scale = args[i].parse().unwrap_or(4); }
            _ => {}
        }
        i += 1;
    }

    eprintln!("contra-compiled: Statically recompiled Contra NES");
    eprintln!("  Mode: {}", mode);

    // ROM data is embedded in the binary at compile time — no file needed
    let mut nes = Nes::embedded();

    eprintln!("  Reset vector: ${:04X}", nes.cpu.pc);

    let seq = build_autoplay_sequence();
    let mut seq_idx = 0;
    let mut frame_num = 0u32;

    match mode {
        "window" => {
            use minifb::{Key, Window, WindowOptions};
            request_realtime_priority();

            let mut window = Window::new(
                "Contra (Compiled)", SCREEN_W * scale, SCREEN_H * scale,
                WindowOptions { resize: false, ..WindowOptions::default() },
            ).expect("Failed to create window");
            window.set_target_fps(60);

            // Pre-run frames for audio buffer
            for _ in 0..4 { nes.run_frame(); frame_num += 1; }

            let audio_buf = nes.bus.apu.audio_buffer();
            let _audio_stream = {
                use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
                let host = cpal::default_host();
                host.default_output_device().and_then(|dev| {
                    let config = cpal::StreamConfig {
                        channels: 1,
                        sample_rate: cpal::SampleRate(apu::SAMPLE_RATE),
                        buffer_size: cpal::BufferSize::Fixed(512),
                    };
                    let buf = audio_buf.clone();
                    let stream = dev.build_output_stream(
                        &config,
                        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                            let mut last = 0.0f32;
                            for sample in data.iter_mut() {
                                if let Some(s) = buf.read() { last = s; }
                                *sample = last;
                            }
                        },
                        |err| eprintln!("Audio error: {}", err),
                        None,
                    ).ok()?;
                    stream.play().ok()?;
                    eprintln!("  Audio: enabled");
                    Some(stream)
                })
            };

            let mut autoplay_done = false;
            let black = vec![0u32; SCREEN_W * SCREEN_H];
            window.update_with_buffer(&black, SCREEN_W, SCREEN_H).unwrap();

            while window.is_open() && !window.is_key_down(Key::Escape) && frame_num < max_frames {
                if !autoplay_done {
                    while seq_idx < seq.len() && seq[seq_idx].frame <= frame_num {
                        nes.set_button(0, seq[seq_idx].button, seq[seq_idx].pressed);
                        seq_idx += 1;
                    }
                    if frame_num > 900 {
                        autoplay_done = true;
                        for b in 0..8 { nes.set_button(0, b, false); }
                    }
                }
                if autoplay_done {
                    nes.set_button(0, BTN_RIGHT, window.is_key_down(Key::Right) || window.is_key_down(Key::D));
                    nes.set_button(0, BTN_LEFT, window.is_key_down(Key::Left) || window.is_key_down(Key::A));
                    nes.set_button(0, BTN_UP, window.is_key_down(Key::Up) || window.is_key_down(Key::W));
                    nes.set_button(0, BTN_DOWN, window.is_key_down(Key::Down) || window.is_key_down(Key::S));
                    nes.set_button(0, BTN_A, window.is_key_down(Key::Z) || window.is_key_down(Key::J));
                    nes.set_button(0, BTN_B, window.is_key_down(Key::X) || window.is_key_down(Key::K));
                    nes.set_button(0, BTN_START, window.is_key_down(Key::Enter));
                    nes.set_button(0, BTN_SELECT, window.is_key_down(Key::Space));
                }
                nes.run_frame();
                frame_num += 1;
                window.update_with_buffer(nes.framebuffer(), SCREEN_W, SCREEN_H).unwrap();
            }
        }

        "benchmark" => {
            nes.audio_enabled = false;
            eprintln!("  Running {} frames...", max_frames);

            // Phase 1: warm-up with rendering
            let t0 = Instant::now();
            for _ in 0..max_frames {
                while seq_idx < seq.len() && seq[seq_idx].frame <= frame_num {
                    nes.set_button(0, seq[seq_idx].button, seq[seq_idx].pressed);
                    seq_idx += 1;
                }
                nes.run_frame();
                frame_num += 1;
            }
            let dt = t0.elapsed();
            let fps = max_frames as f64 / dt.as_secs_f64();
            eprintln!("\n  Benchmark: {} frames in {:.1}ms = {:.0} fps", max_frames, dt.as_secs_f64() * 1000.0, fps);
            eprintln!("  Speed: {:.1}x realtime", fps / 60.0);
            eprintln!("  {:.0} µs/frame", dt.as_micros() as f64 / max_frames as f64);
            write_bmp("compiled-frame.bmp", nes.framebuffer(), SCREEN_W as u32, SCREEN_H as u32);
        }

        "headless" => {
            for _ in 0..max_frames {
                while seq_idx < seq.len() && seq[seq_idx].frame <= frame_num {
                    nes.set_button(0, seq[seq_idx].button, seq[seq_idx].pressed);
                    seq_idx += 1;
                }
                nes.run_frame();
                frame_num += 1;
            }
            eprintln!("  Done: {} frames", frame_num);
        }

        _ => eprintln!("Unknown mode. Use --window, --benchmark, or --headless"),
    }
}
