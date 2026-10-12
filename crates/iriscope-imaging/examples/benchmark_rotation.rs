//! Full-resolution RGB rotation timings, including destination allocation.
use image::RgbImage;
use iriscope_imaging::rotate_rgb;
use std::{hint::black_box, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1_280_u32, 1_024_u32);
    let mut rgb = vec![0_u8; usize::try_from(width)? * usize::try_from(height)? * 3];
    let mut noise = 0x1234_5678_u32;
    for pixel in rgb.chunks_exact_mut(3) {
        noise ^= noise << 13;
        noise ^= noise >> 17;
        noise ^= noise << 5;
        pixel.copy_from_slice(&noise.to_le_bytes()[..3]);
    }
    let source = RgbImage::from_raw(width, height, rgb).ok_or("Invalid RGB source")?;
    for angle in [0, 37, 90, 135, 180, 270, 315] {
        let mut timings = Vec::new();
        let mut checksum = 0_u64;
        let mut dimensions = (0, 0);
        for run in 0..23 {
            let input = source.clone();
            let start = Instant::now();
            let output = black_box(rotate_rgb(black_box(input), black_box(angle)));
            let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
            if run >= 3 {
                timings.push(elapsed);
            }
            dimensions = output.dimensions();
            checksum = output.as_raw().iter().fold(0_u64, |sum, byte| {
                sum.wrapping_mul(31).wrapping_add(u64::from(*byte))
            });
        }
        timings.sort_by(f64::total_cmp);
        println!(
            "angle={angle} resolution={width}x{height} output={}x{} runs={} median_ms={:.3} p95_ms={:.3} checksum={checksum:016x}",
            dimensions.0,
            dimensions.1,
            timings.len(),
            timings[timings.len() / 2],
            timings[timings.len() * 95 / 100]
        );
    }
    Ok(())
}
