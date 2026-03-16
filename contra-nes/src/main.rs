// contra-nes: Native Rust Contra
//
// Game logic runs through a native Rust Game module.
// ROM data is compiled into the binary — no external .nes file needed.
// Terminal renderer with truecolor ANSI. Headless benchmark mode.
// Autoplay with Konami code for 30 lives.

mod apu;
mod cartridge;
mod data;
mod enemies;
mod game;
mod level;
mod player;
mod renderer;

use std::env;
use std::fs;
use std::io::{self, Write};
use std::os::unix::io::AsRawFd;
use std::time::Instant;

/// Request realtime thread priority to minimize OS scheduling jitter.
/// On macOS: uses Mach thread_policy_set with THREAD_TIME_CONSTRAINT_POLICY.
/// On Linux: uses sched_setscheduler with SCHED_RR (requires root or CAP_SYS_NICE).
/// Fails silently — realtime priority is a nice-to-have, not required.
fn request_realtime_priority() {
    #[cfg(target_os = "macos")]
    {
        // Mach realtime thread scheduling
        #[repr(C)]
        struct ThreadTimeConstraintPolicy {
            period: u32,
            computation: u32,
            constraint: u32,
            preemptible: i32,
        }

        const THREAD_TIME_CONSTRAINT_POLICY: u32 = 2;

        extern "C" {
            fn mach_thread_self() -> u32;
            fn thread_policy_set(
                thread: u32,
                flavor: u32,
                policy_info: *const ThreadTimeConstraintPolicy,
                count: u32,
            ) -> i32;
        }

        let policy = ThreadTimeConstraintPolicy {
            period: 16_666_667,
            computation: 2_000_000,
            constraint: 4_000_000,
            preemptible: 1,
        };

        let ret = unsafe {
            thread_policy_set(
                mach_thread_self(),
                THREAD_TIME_CONSTRAINT_POLICY,
                &policy as *const ThreadTimeConstraintPolicy,
                4,
            )
        };

        if ret == 0 {
            eprintln!("  Thread: realtime priority (macOS THREAD_TIME_CONSTRAINT_POLICY)");
        } else {
            eprintln!("  Thread: realtime priority failed (ret={}), using default", ret);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let param = libc::sched_param { sched_priority: 10 };
        let ret = unsafe { libc::sched_setscheduler(0, libc::SCHED_RR, &param) };
        if ret == 0 {
            eprintln!("  Thread: realtime priority (Linux SCHED_RR, priority=10)");
        } else {
            eprintln!("  Thread: realtime priority unavailable (need root/CAP_SYS_NICE), using default");
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        eprintln!("  Thread: realtime priority not supported on this platform");
    }
}

use crate::game::Game;

const SCREEN_W: usize = 256;
const SCREEN_H: usize = 240;

// Button indices (NES standard)
const BTN_A: u8 = 0;
const BTN_B: u8 = 1;
const BTN_SELECT: u8 = 2;
const BTN_START: u8 = 3;
const BTN_UP: u8 = 4;
const BTN_DOWN: u8 = 5;
const BTN_LEFT: u8 = 6;
const BTN_RIGHT: u8 = 7;

// ── Input sequence for autoplay ──

struct InputEvent {
    frame: u32,
    button: u8,
    pressed: bool,
}

fn build_autoplay_sequence() -> Vec<InputEvent> {
    let mut seq = Vec::new();
    let mut f = 250u32;

    // Helper: tap a button
    let mut tap = |seq: &mut Vec<InputEvent>, btn: u8, dur: u32| {
        seq.push(InputEvent { frame: f, button: btn, pressed: true });
        seq.push(InputEvent { frame: f + dur, button: btn, pressed: false });
        f += dur + 6;
    };

    // Konami code
    tap(&mut seq, BTN_UP, 4); tap(&mut seq, BTN_UP, 4);
    tap(&mut seq, BTN_DOWN, 4); tap(&mut seq, BTN_DOWN, 4);
    tap(&mut seq, BTN_LEFT, 4); tap(&mut seq, BTN_RIGHT, 4);
    tap(&mut seq, BTN_LEFT, 4); tap(&mut seq, BTN_RIGHT, 4);
    tap(&mut seq, BTN_B, 4); tap(&mut seq, BTN_A, 4);
    tap(&mut seq, BTN_START, 4);

    // Hold right after game starts
    let gs = f + 200;
    seq.push(InputEvent { frame: gs, button: BTN_RIGHT, pressed: true });

    // Periodic jump + shoot
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

// ── Terminal renderer ──

fn render_to_terminal(fb: &[u8], scale: usize) {
    let mut out = String::with_capacity(SCREEN_W / scale * SCREEN_H / scale * 20);

    // Move cursor to top-left
    out.push_str("\x1b[H");

    // Use half-block rendering: each character cell = 2 rows of pixels
    let rows = SCREEN_H / scale;
    let cols = SCREEN_W / scale;

    for row in (0..rows).step_by(2) {
        for col in 0..cols {
            let px = col * scale;
            let py_top = row * scale;
            let py_bot = (row + 1) * scale;

            let top_off = (py_top * SCREEN_W + px) * 3;
            let (tr, tg, tb) = if top_off + 2 < fb.len() {
                (fb[top_off], fb[top_off + 1], fb[top_off + 2])
            } else {
                (0, 0, 0)
            };

            let bot_off = (py_bot * SCREEN_W + px) * 3;
            let (br, bg_, bb) = if bot_off + 2 < fb.len() {
                (fb[bot_off], fb[bot_off + 1], fb[bot_off + 2])
            } else {
                (0, 0, 0)
            };

            // upper half block: fg = top pixel, bg = bottom pixel
            out.push_str(&format!(
                "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m\u{2580}",
                tr, tg, tb, br, bg_, bb
            ));
        }
        out.push_str("\x1b[0m\n");
    }

    let stdout = io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(out.as_bytes());
    let _ = lock.flush();
}

// ── BMP writer ──

fn write_bmp(path: &str, px: &[u8], w: u32, h: u32) {
    let stride = ((w * 3 + 3) / 4 * 4) as usize;
    let data_sz = stride * h as usize;
    let mut f = fs::File::create(path).unwrap();
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
            let s = (y * w as usize + x) * 3;
            row[x*3] = px[s+2]; row[x*3+1] = px[s+1]; row[x*3+2] = px[s];
        }
        f.write_all(&row).unwrap();
    }
}

// ── PPM writer (for quick terminal viewing) ──

fn write_ppm(path: &str, px: &[u8], w: u32, h: u32) {
    let mut f = fs::File::create(path).unwrap();
    write!(f, "P6\n{} {}\n255\n", w, h).unwrap();
    f.write_all(&px[..w as usize * h as usize * 3]).unwrap();
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut mode = "window"; // window, benchmark, headless, terminal, export
    let mut max_frames = u32::MAX; // unlimited for window/play modes
    let mut scale = 4usize;
    let mut export_dir = String::from(".");
    let mut export_interval = 500u32;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--window" | "-w" => { mode = "window"; }
            "--benchmark" | "-b" => { mode = "benchmark"; }
            "--headless" => { mode = "headless"; }
            "--terminal" | "-t" => { mode = "terminal"; }
            "--play" | "-p" => { mode = "play"; }
            "--export" | "-e" => { mode = "export"; }
            "--pipe" => { mode = "pipe"; }
            "--frames" | "-n" => { i += 1; max_frames = args[i].parse().unwrap_or(6000); }
            "--scale" | "-s" => { i += 1; scale = args[i].parse().unwrap_or(4); }
            "--dir" | "-d" => { i += 1; export_dir = args[i].clone(); }
            "--interval" => { i += 1; export_interval = args[i].parse().unwrap_or(500); }
            _ => {}
        }
        i += 1;
    }

    eprintln!("contra-nes: Native Rust Contra");
    eprintln!("  Mode: {}", mode);

    // Game data is compiled into the binary
    let mut game = Game::new(data::PRG_DATA, data::PRG_BANKS);

    eprintln!("  PRG: {} banks x 16KB = {}KB (compiled in)", data::PRG_BANKS, data::PRG_BANKS * 16);

    let seq = build_autoplay_sequence();
    let mut seq_idx = 0;
    let mut frame_num = 0u32;

    match mode {
        "window" => {
            use minifb::{Key, Window, WindowOptions};

            request_realtime_priority();

            let win_w = SCREEN_W * scale;
            let win_h = SCREEN_H * scale;
            let mut window = Window::new(
                "Contra NES",
                win_w, win_h,
                WindowOptions {
                    resize: false,
                    scale_mode: minifb::ScaleMode::AspectRatioStretch,
                    ..WindowOptions::default()
                },
            ).expect("Failed to create window");

            window.set_target_fps(60);

            // Start audio output stream
            let audio_buf = game.audio_buffer();
            let _audio_stream = {
                use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
                let host = cpal::default_host();
                let device = host.default_output_device();
                device.and_then(|dev| {
                    let config = cpal::StreamConfig {
                        channels: 1,
                        sample_rate: cpal::SampleRate(apu::SAMPLE_RATE),
                        buffer_size: cpal::BufferSize::Default,
                    };
                    let buf = audio_buf.clone();
                    let stream = dev.build_output_stream(
                        &config,
                        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                            for sample in data.iter_mut() {
                                *sample = buf.read();
                            }
                        },
                        |err| eprintln!("Audio error: {}", err),
                        None,
                    ).ok()?;
                    stream.play().ok()?;
                    eprintln!("  Audio: enabled (44.1kHz mono)");
                    Some(stream)
                })
            };

            let mut fb32 = vec![0u32; SCREEN_W * SCREEN_H];
            let mut autoplay_done = false;

            // Render initial black frame so update_with_buffer refreshes key state
            window.update_with_buffer(&fb32, SCREEN_W, SCREEN_H).unwrap();

            while window.is_open() && !window.is_key_down(Key::Escape) && frame_num < max_frames {
                if !autoplay_done {
                    while seq_idx < seq.len() && seq[seq_idx].frame <= frame_num {
                        game.set_button(0, seq[seq_idx].button, seq[seq_idx].pressed);
                        seq_idx += 1;
                    }
                    if frame_num > 900 {
                        autoplay_done = true;
                        for b in 0..8 { game.set_button(0, b, false); }
                    }
                }

                if autoplay_done {
                    game.set_button(0, BTN_RIGHT, window.is_key_down(Key::Right) || window.is_key_down(Key::D));
                    game.set_button(0, BTN_LEFT, window.is_key_down(Key::Left) || window.is_key_down(Key::A));
                    game.set_button(0, BTN_UP, window.is_key_down(Key::Up) || window.is_key_down(Key::W));
                    game.set_button(0, BTN_DOWN, window.is_key_down(Key::Down) || window.is_key_down(Key::S));
                    game.set_button(0, BTN_A, window.is_key_down(Key::Z) || window.is_key_down(Key::J));
                    game.set_button(0, BTN_B, window.is_key_down(Key::X) || window.is_key_down(Key::K));
                    game.set_button(0, BTN_START, window.is_key_down(Key::Enter));
                    game.set_button(0, BTN_SELECT, window.is_key_down(Key::Space));
                }

                game.update();
                game.render();
                frame_num += 1;

                let fb = game.framebuffer();
                for i in 0..SCREEN_W * SCREEN_H {
                    let o = i * 3;
                    fb32[i] = (fb[o] as u32) << 16 | (fb[o + 1] as u32) << 8 | fb[o + 2] as u32;
                }

                window.update_with_buffer(&fb32, SCREEN_W, SCREEN_H).unwrap();
            }
            eprintln!("  Played {} frames", frame_num);
        }

        "benchmark" => {
            eprintln!("  Running {} frames...", max_frames);
            let t0 = Instant::now();

            for _ in 0..max_frames {
                while seq_idx < seq.len() && seq[seq_idx].frame <= frame_num {
                    game.set_button(0, seq[seq_idx].button, seq[seq_idx].pressed);
                    seq_idx += 1;
                }
                game.update();
                frame_num += 1;
            }

            let dt = t0.elapsed();
            let fps = max_frames as f64 / dt.as_secs_f64();
            eprintln!("\n  Benchmark: {} frames in {:.1}ms = {:.0} fps",
                max_frames, dt.as_secs_f64() * 1000.0, fps);
            eprintln!("  Speed: {:.1}x NES real-time (vs 60fps)", fps / 60.0);
            eprintln!("  {:.0} ns/frame", dt.as_nanos() as f64 / max_frames as f64);

            let out = format!("{}/emu-frame-final.bmp", export_dir);
            write_bmp(&out, game.framebuffer(), SCREEN_W as u32, SCREEN_H as u32);
            eprintln!("  Saved {}", out);
        }

        "terminal" => {
            print!("\x1b[2J\x1b[?25l");
            io::stdout().flush().unwrap();

            let frame_time = std::time::Duration::from_micros(16667);

            for _ in 0..max_frames {
                let t0 = Instant::now();

                while seq_idx < seq.len() && seq[seq_idx].frame <= frame_num {
                    game.set_button(0, seq[seq_idx].button, seq[seq_idx].pressed);
                    seq_idx += 1;
                }
                game.update();
                frame_num += 1;

                if frame_num % 2 == 0 {
                    render_to_terminal(game.framebuffer(), scale);
                }

                let elapsed = t0.elapsed();
                if elapsed < frame_time {
                    std::thread::sleep(frame_time - elapsed);
                }
            }

            print!("\x1b[?25h\x1b[0m");
            io::stdout().flush().unwrap();
        }

        "headless" => {
            eprintln!("  Running {} frames headless...", max_frames);
            for _ in 0..max_frames {
                while seq_idx < seq.len() && seq[seq_idx].frame <= frame_num {
                    game.set_button(0, seq[seq_idx].button, seq[seq_idx].pressed);
                    seq_idx += 1;
                }
                game.update();
                frame_num += 1;
            }
            eprintln!("  Done. Frame count: {}", frame_num);
        }

        "play" => {
            eprintln!("  Controls: WASD=move, J=jump(A), K=shoot(B), Enter=Start, Q=quit");
            eprintln!("  Starting in 1s...");
            std::thread::sleep(std::time::Duration::from_secs(1));

            let stdin_fd = io::stdin().as_raw_fd();
            let orig_termios = unsafe {
                let mut t = std::mem::zeroed::<libc::termios>();
                libc::tcgetattr(stdin_fd, &mut t);
                let orig = t;
                libc::cfmakeraw(&mut t);
                t.c_cc[libc::VMIN] = 0;
                t.c_cc[libc::VTIME] = 0;
                libc::tcsetattr(stdin_fd, libc::TCSANOW, &t);
                orig
            };

            print!("\x1b[2J\x1b[?25l");
            io::stdout().flush().unwrap();

            let frame_time = std::time::Duration::from_micros(16667);
            let mut running = true;
            let mut input_buf = [0u8; 32];
            let mut autoplay_done = false;

            while running && frame_num < max_frames {
                let t0 = Instant::now();

                if !autoplay_done {
                    while seq_idx < seq.len() && seq[seq_idx].frame <= frame_num {
                        game.set_button(0, seq[seq_idx].button, seq[seq_idx].pressed);
                        seq_idx += 1;
                    }
                    if frame_num > 900 {
                        autoplay_done = true;
                        for b in 0..8 { game.set_button(0, b, false); }
                        game.set_button(0, BTN_RIGHT, true);
                    }
                }

                if autoplay_done {
                    let n = unsafe {
                        libc::read(stdin_fd, input_buf.as_mut_ptr() as *mut libc::c_void, input_buf.len())
                    };
                    if n > 0 {
                        for &b in &input_buf[..n as usize] {
                            match b {
                                b'q' | 3 => { running = false; }
                                b'w' => { game.set_button(0, BTN_UP, true); }
                                b's' => { game.set_button(0, BTN_DOWN, true); }
                                b'a' => { game.set_button(0, BTN_LEFT, true); game.set_button(0, BTN_RIGHT, false); }
                                b'd' => { game.set_button(0, BTN_RIGHT, true); game.set_button(0, BTN_LEFT, false); }
                                b'j' => { game.set_button(0, BTN_A, true); }
                                b'k' => { game.set_button(0, BTN_B, true); }
                                b'\r' | b'\n' => { game.set_button(0, BTN_START, true); }
                                b' ' => { game.set_button(0, BTN_SELECT, true); }
                                b'W' => { game.set_button(0, BTN_UP, false); }
                                b'S' => { game.set_button(0, BTN_DOWN, false); }
                                b'A' => { game.set_button(0, BTN_LEFT, false); }
                                b'D' => { game.set_button(0, BTN_RIGHT, false); }
                                b'J' => { game.set_button(0, BTN_A, false); }
                                b'K' => { game.set_button(0, BTN_B, false); }
                                _ => {}
                            }
                        }
                    }
                    if frame_num % 8 == 0 {
                        game.set_button(0, BTN_A, false);
                        game.set_button(0, BTN_B, false);
                        game.set_button(0, BTN_START, false);
                        game.set_button(0, BTN_SELECT, false);
                        game.set_button(0, BTN_UP, false);
                        game.set_button(0, BTN_DOWN, false);
                    }
                }

                game.update();
                frame_num += 1;

                if frame_num % 2 == 0 {
                    render_to_terminal(game.framebuffer(), scale);
                }

                let elapsed = t0.elapsed();
                if elapsed < frame_time {
                    std::thread::sleep(frame_time - elapsed);
                }
            }

            unsafe { libc::tcsetattr(stdin_fd, libc::TCSANOW, &orig_termios); }
            print!("\x1b[?25h\x1b[0m\n");
            io::stdout().flush().unwrap();
            eprintln!("  Played {} frames", frame_num);
        }

        "export" => {
            eprintln!("  Exporting frames every {} to {}/", export_interval, export_dir);
            for _ in 0..max_frames {
                while seq_idx < seq.len() && seq[seq_idx].frame <= frame_num {
                    game.set_button(0, seq[seq_idx].button, seq[seq_idx].pressed);
                    seq_idx += 1;
                }
                game.update();

                if frame_num % export_interval == 0 {
                    let path = format!("{}/emu-frame-{:05}.bmp", export_dir, frame_num);
                    write_bmp(&path, game.framebuffer(), SCREEN_W as u32, SCREEN_H as u32);
                    let ppm_path = format!("{}/emu-frame-{:05}.ppm", export_dir, frame_num);
                    write_ppm(&ppm_path, game.framebuffer(), SCREEN_W as u32, SCREEN_H as u32);
                    eprintln!("  Frame {}: saved", frame_num);
                }
                frame_num += 1;
            }
            eprintln!("  Exported {} frames", frame_num);
        }

        "pipe" => {
            eprintln!("  Piping {} frames as raw RGB24 to stdout...", max_frames);
            let stdout = io::stdout();
            let mut out = io::BufWriter::new(stdout.lock());

            for _ in 0..max_frames {
                while seq_idx < seq.len() && seq[seq_idx].frame <= frame_num {
                    game.set_button(0, seq[seq_idx].button, seq[seq_idx].pressed);
                    seq_idx += 1;
                }
                game.update();
                out.write_all(&game.framebuffer()[..SCREEN_W * SCREEN_H * 3]).unwrap();
                frame_num += 1;
            }
            out.flush().unwrap();
            eprintln!("  Piped {} frames", frame_num);
        }

        _ => {
            eprintln!("Unknown mode: {}", mode);
            eprintln!("  Modes: --benchmark, --terminal, --play, --export, --headless, --pipe");
        }
    }

    eprintln!("Done.");
}
