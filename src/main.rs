#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dioxus::desktop::tao::window::Icon;
use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
use llamamanager::app::{App, Bootstrap, set_bootstrap};
use tracing_subscriber::EnvFilter;

const ICON_SIZE: usize = 64;

fn app_icon() -> Option<Icon> {
    let mut rgba = vec![0_u8; ICON_SIZE * ICON_SIZE * 4];
    let center = (ICON_SIZE as f32 - 1.0) / 2.0;

    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let radius = (dx * dx + dy * dy).sqrt();

            if radius > 30.0 {
                continue;
            }

            let mut color = [9, 0, 20, 255];

            if radius >= 27.8 {
                color = rim_color(y);
            } else if (8..39).contains(&y) && radius <= 25.8 {
                let local_y = y - 8;
                if local_y % 6 <= 3 {
                    color = sunset_color(local_y as f32 / 31.0);
                }
            }

            if y >= 38 && radius <= 26.5 && grid_pixel(x, y) {
                color = grid_color(x);
            }

            put_pixel(&mut rgba, x, y, color);
        }
    }

    draw_wave(&mut rgba);

    Icon::from_rgba(rgba, ICON_SIZE as u32, ICON_SIZE as u32).ok()
}

fn rim_color(y: usize) -> [u8; 4] {
    let t = y as f32 / (ICON_SIZE - 1) as f32;
    if t < 0.48 {
        mix([255, 153, 0, 255], [255, 0, 255, 255], t / 0.48)
    } else {
        mix([255, 0, 255, 255], [0, 255, 255, 255], (t - 0.48) / 0.52)
    }
}

fn sunset_color(t: f32) -> [u8; 4] {
    if t < 0.5 {
        mix([255, 230, 80, 255], [255, 95, 140, 255], t * 2.0)
    } else {
        mix([255, 95, 140, 255], [205, 0, 255, 255], (t - 0.5) * 2.0)
    }
}

fn grid_color(x: usize) -> [u8; 4] {
    if x < ICON_SIZE / 2 {
        [255, 0, 255, 255]
    } else {
        [0, 230, 255, 255]
    }
}

fn grid_pixel(x: usize, y: usize) -> bool {
    const HORIZON: usize = 39;
    const BOTTOM: usize = 58;
    const HORIZONTAL_LINES: [usize; 5] = [40, 43, 47, 52, 58];
    const BOTTOM_X: [i32; 7] = [10, 17, 24, 32, 40, 47, 54];

    if HORIZONTAL_LINES.contains(&y) {
        return true;
    }

    if !(HORIZON..=BOTTOM).contains(&y) {
        return false;
    }

    let progress = (y - HORIZON) as f32 / (BOTTOM - HORIZON) as f32;
    BOTTOM_X.iter().any(|bottom_x| {
        let projected = 32.0 + (*bottom_x as f32 - 32.0) * progress;
        (x as f32 - projected).abs() <= 0.6
    })
}

fn draw_wave(rgba: &mut [u8]) {
    for step in 0..=96 {
        let t = step as f32 / 96.0;
        let one_minus_t = 1.0 - t;

        let x = one_minus_t * one_minus_t * 10.0 + 2.0 * one_minus_t * t * 27.0 + t * t * 40.0;
        let y = one_minus_t * one_minus_t * 43.0 + 2.0 * one_minus_t * t * 43.0 + t * t * 34.0;
        let color = mix([255, 0, 255, 255], [80, 170, 255, 255], t);
        draw_neon_point(rgba, x, y, color);
    }

    for step in 0..=72 {
        let t = step as f32 / 72.0;
        let one_minus_t = 1.0 - t;

        let x = one_minus_t * one_minus_t * 40.0 + 2.0 * one_minus_t * t * 51.0 + t * t * 46.0;
        let y = one_minus_t * one_minus_t * 34.0 + 2.0 * one_minus_t * t * 31.0 + t * t * 40.0;
        draw_neon_point(rgba, x, y, [0, 245, 255, 255]);
    }

    for step in 0..=72 {
        let t = step as f32 / 72.0;
        let one_minus_t = 1.0 - t;

        let x = one_minus_t * one_minus_t * 46.0 + 2.0 * one_minus_t * t * 40.0 + t * t * 52.0;
        let y = one_minus_t * one_minus_t * 40.0 + 2.0 * one_minus_t * t * 37.0 + t * t * 44.0;
        draw_neon_point(rgba, x, y, [0, 245, 255, 255]);
    }
}

fn draw_neon_point(rgba: &mut [u8], x: f32, y: f32, color: [u8; 4]) {
    let x = x.round() as i32;
    let y = y.round() as i32;

    for offset_y in -1..=1 {
        for offset_x in -1..=1 {
            let px = x + offset_x;
            let py = y + offset_y;
            if px < 0 || py < 0 || px >= ICON_SIZE as i32 || py >= ICON_SIZE as i32 {
                continue;
            }

            let dx = px as f32 - (ICON_SIZE as f32 - 1.0) / 2.0;
            let dy = py as f32 - (ICON_SIZE as f32 - 1.0) / 2.0;
            if (dx * dx + dy * dy).sqrt() > 27.0 {
                continue;
            }

            let brightness = if offset_x == 0 && offset_y == 0 {
                1.0
            } else {
                0.58
            };
            blend_pixel(rgba, px as usize, py as usize, color, brightness);
        }
    }
}

fn put_pixel(rgba: &mut [u8], x: usize, y: usize, color: [u8; 4]) {
    let index = (y * ICON_SIZE + x) * 4;
    rgba[index..index + 4].copy_from_slice(&color);
}

fn blend_pixel(rgba: &mut [u8], x: usize, y: usize, color: [u8; 4], strength: f32) {
    let index = (y * ICON_SIZE + x) * 4;
    for channel in 0..3 {
        let current = rgba[index + channel] as f32;
        let target = color[channel] as f32;
        rgba[index + channel] = (current + (target - current) * strength).round() as u8;
    }
    rgba[index + 3] = 255;
}

fn mix(from: [u8; 4], to: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    let mut result = [0_u8; 4];
    for channel in 0..4 {
        result[channel] =
            (from[channel] as f32 + (to[channel] as f32 - from[channel] as f32) * t).round() as u8;
    }
    result
}

fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();

    let bootstrap = Bootstrap::initialize().map_err(|error| {
        tracing::error!(%error, "application bootstrap failed");
        error.to_string()
    });
    set_bootstrap(bootstrap);

    let window = WindowBuilder::new()
        .with_title("LlamaWave")
        .with_window_icon(app_icon())
        .with_inner_size(LogicalSize::new(1440.0, 900.0))
        .with_min_inner_size(LogicalSize::new(1100.0, 700.0));

    let config = Config::new()
        .with_window(window)
        .with_menu(None)
        .with_background_color((7, 3, 18, 255))
        .with_disable_context_menu(true);

    dioxus::LaunchBuilder::new().with_cfg(config).launch(App);
}
