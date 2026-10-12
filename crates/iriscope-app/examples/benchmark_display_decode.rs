//! Compare JPEG-to-display decoding with and without an intermediate RGB copy.
#[path = "../src/pixel_buffer.rs"]
mod pixel_buffer;

use iriscope_imaging::{decode_mjpeg_to_rgb8, encode_rgb8_jpeg};
use slint::{Rgb8Pixel, SharedPixelBuffer};
use std::{hint::black_box, time::Instant};

fn copied_decode(jpeg: &[u8]) -> Result<SharedPixelBuffer<Rgb8Pixel>, Box<dyn std::error::Error>> {
    let (width, height, rgb) = decode_mjpeg_to_rgb8(jpeg)?;
    Ok(SharedPixelBuffer::clone_from_slice(&rgb, width, height))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280_u32, 1024_u32);
    let mut rgb = vec![0_u8; usize::try_from(width)? * usize::try_from(height)? * 3];
    let mut noise = 0x1234_5678_u32;
    for pixel in rgb.chunks_exact_mut(3) {
        noise ^= noise << 13;
        noise ^= noise >> 17;
        noise ^= noise << 5;
        pixel.copy_from_slice(&noise.to_le_bytes()[..3]);
    }
    let jpeg = encode_rgb8_jpeg(&rgb, width, height, 85)?;
    let copied = copied_decode(&jpeg)?;
    let direct = pixel_buffer::decode_mjpeg_pixels(&jpeg)?;
    assert_eq!(copied.as_bytes(), direct.as_bytes());
    drop((copied, direct, rgb));

    // Alternate order to reduce bias from CPU frequency and competing work.
    let mut copied_times = Vec::new();
    let mut direct_times = Vec::new();
    for iteration in 0..110 {
        for direct_first in [iteration % 2 == 0, iteration % 2 != 0] {
            let started = Instant::now();
            if direct_first {
                black_box(pixel_buffer::decode_mjpeg_pixels(black_box(&jpeg))?);
            } else {
                black_box(copied_decode(black_box(&jpeg))?);
            }
            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
            if iteration >= 10 {
                if direct_first {
                    direct_times.push(elapsed_ms);
                } else {
                    copied_times.push(elapsed_ms);
                }
            }
        }
    }
    println!(
        "resolution={width}x{height} jpeg_bytes={} runs=100",
        jpeg.len()
    );
    for (name, mut timings) in [("copied", copied_times), ("direct", direct_times)] {
        timings.sort_by(f64::total_cmp);
        println!(
            "pipeline={name} median_ms={:.3} p95_ms={:.3}",
            timings[50], timings[95]
        );
    }
    println!(
        "intermediate_rgb_bytes_avoided={}",
        u64::from(width) * u64::from(height) * 3
    );
    Ok(())
}
