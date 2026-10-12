//! Rotation geometry shared by display pixels and exported annotations.

use image::RgbImage;

use crate::ImagingError;

/// Full image bounds after a clockwise rotation, without cropping the source.
#[derive(Clone, Copy, Debug)]
pub struct RotationGeometry {
    width: f64,
    height: f64,
    cosine: f64,
    sine: f64,
    output_width: u32,
    output_height: u32,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Bounded image dimensions.
impl RotationGeometry {
    /// Computes the enclosing pixel bounds. Angles are normalized to 0..359°.
    #[must_use]
    pub fn new(width: u32, height: u32, degrees: i32) -> Self {
        let (cosine, sine) = match degrees.rem_euclid(360) {
            0 => (1.0, 0.0),
            90 => (0.0, 1.0),
            180 => (-1.0, 0.0),
            270 => (0.0, -1.0),
            angle => {
                let radians = f64::from(angle).to_radians();
                (radians.cos(), radians.sin())
            }
        };
        let width = f64::from(width);
        let height = f64::from(height);
        Self {
            width,
            height,
            cosine,
            sine,
            output_width: (width * cosine.abs() + height * sine.abs()).ceil() as u32,
            output_height: (width * sine.abs() + height * cosine.abs()).ceil() as u32,
        }
    }

    /// Dimensions of the enclosing rotated image.
    #[must_use]
    pub fn dimensions(self) -> (u32, u32) {
        (self.output_width, self.output_height)
    }

    /// Maps normalized source coordinates into normalized output coordinates.
    #[must_use]
    pub fn project(self, x: f32, y: f32, mirror: bool) -> (f32, f32) {
        let dx = (f64::from(x) - 0.5) * self.width;
        let dy = (f64::from(y) - 0.5) * self.height;
        let x = ((dx * self.cosine - dy * self.sine) / f64::from(self.output_width.max(1)) + 0.5)
            as f32;
        let y = ((dx * self.sine + dy * self.cosine) / f64::from(self.output_height.max(1)) + 0.5)
            as f32;
        (if mirror { 1.0 - x } else { x }, y)
    }

    fn source_pixel(self, x: u32, y: u32) -> (f64, f64) {
        let dx = f64::from(x) + 0.5 - f64::from(self.output_width) / 2.0;
        let dy = f64::from(y) + 0.5 - f64::from(self.output_height) / 2.0;
        (
            dx * self.cosine + dy * self.sine + self.width / 2.0 - 0.5,
            -dx * self.sine + dy * self.cosine + self.height / 2.0 - 0.5,
        )
    }
}

/// Rotates RGB pixels clockwise with bilinear sampling and black outer corners.
#[must_use]
pub fn rotate_rgb(image: RgbImage, degrees: i32) -> RgbImage {
    match degrees.rem_euclid(360) {
        0 => image,
        90 => image::imageops::rotate90(&image),
        180 => image::imageops::rotate180(&image),
        270 => image::imageops::rotate270(&image),
        _ => rotate_noncardinal(&image, degrees),
    }
}

fn rotate_noncardinal(image: &RgbImage, degrees: i32) -> RgbImage {
    let geometry = RotationGeometry::new(image.width(), image.height(), degrees);
    let (width, height) = geometry.dimensions();
    let mut output = RgbImage::new(width, height);
    if width == 0 || height == 0 {
        return output;
    }
    render_rotation(
        image.as_raw(),
        image.width() as usize,
        image.height() as usize,
        geometry,
        false,
        None,
        output.as_mut(),
    );
    output
}

/// Applies channel adjustments, a clockwise rotation and an optional horizontal
/// mirror directly into the caller's RGB8 destination.
///
/// `channel_map` maps source channels before bilinear interpolation, preserving
/// the result of adjusting the original image and then rotating it. The allocator
/// runs after source validation and receives the enclosing output dimensions.
/// No intermediate full-size image or post-rotation mirror is allocated.
///
/// # Errors
///
/// Returns [`ImagingError::InvalidBufferSize`] for an invalid source, overflowing
/// output dimensions, or a destination whose length does not match its dimensions.
pub fn transform_rgb_to_buffer<B: AsMut<[u8]>>(
    source: &[u8],
    width: u32,
    height: u32,
    degrees: i32,
    mirror: bool,
    channel_map: Option<&[u8; 256]>,
    allocate: impl FnOnce(u32, u32) -> B,
) -> Result<(u32, u32, B), ImagingError> {
    crate::validate_rgb8(source, width, height)?;
    let geometry = RotationGeometry::new(width, height, degrees);
    let (output_width, output_height) = geometry.dimensions();
    let expected = usize::try_from(output_width)
        .ok()
        .and_then(|width| width.checked_mul(output_height as usize))
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or(ImagingError::InvalidBufferSize)?;
    let mut output = allocate(output_width, output_height);
    if output.as_mut().len() != expected {
        return Err(ImagingError::InvalidBufferSize);
    }
    let angle = degrees.rem_euclid(360);
    if angle % 90 == 0 {
        render_cardinal(
            source,
            width as usize,
            height as usize,
            angle,
            mirror,
            channel_map,
            output.as_mut(),
        );
    } else {
        render_rotation(
            source,
            width as usize,
            height as usize,
            geometry,
            mirror,
            channel_map,
            output.as_mut(),
        );
    }
    Ok((output_width, output_height, output))
}

fn render_cardinal(
    source: &[u8],
    width: usize,
    height: usize,
    angle: i32,
    mirror: bool,
    channel_map: Option<&[u8; 256]>,
    output: &mut [u8],
) {
    let output_width = if angle % 180 == 0 { width } else { height };
    for (y, row) in output.chunks_exact_mut(output_width * 3).enumerate() {
        for (x, pixel) in row.chunks_exact_mut(3).enumerate() {
            let x = if mirror { output_width - 1 - x } else { x };
            let (sx, sy) = match angle {
                90 => (y, height - 1 - x),
                180 => (width - 1 - x, height - 1 - y),
                270 => (width - 1 - y, x),
                _ => (x, y),
            };
            let offset = (sy * width + sx) * 3;
            pixel.copy_from_slice(&source[offset..offset + 3]);
            if let Some(map) = channel_map {
                for channel in pixel {
                    *channel = map[usize::from(*channel)];
                }
            }
        }
    }
}

fn render_rotation(
    source: &[u8],
    width: usize,
    height: usize,
    geometry: RotationGeometry,
    mirror: bool,
    channel_map: Option<&[u8; 256]>,
    output: &mut [u8],
) {
    let output_width = geometry.output_width as usize;
    for (y, row) in output.chunks_exact_mut(output_width * 3).enumerate() {
        for (x, pixel) in row.chunks_exact_mut(3).enumerate() {
            let x = if mirror { output_width - 1 - x } else { x };
            #[allow(clippy::cast_possible_truncation)]
            // Indices are bounded by u32 image dimensions.
            let (sx, sy) = geometry.source_pixel(x as u32, y as u32);
            pixel.copy_from_slice(&sample(source, width, height, sx, sy, channel_map));
        }
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)] // Coordinates are within validated u32 dimensions, and colors are clamped.
fn sample(
    source: &[u8],
    width: usize,
    height: usize,
    x: f64,
    y: f64,
    channel_map: Option<&[u8; 256]>,
) -> [u8; 3] {
    if width == 0
        || height == 0
        || x < -0.5
        || y < -0.5
        || x > width as f64 - 0.5
        || y > height as f64 - 0.5
    {
        return [0; 3];
    }
    let x = x.clamp(0.0, (width - 1) as f64);
    let y = y.clamp(0.0, (height - 1) as f64);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let fx = x - x0 as f64;
    let fy = y - y0 as f64;
    let top_left = (y0 * width + x0) * 3;
    let top_right = (y0 * width + x1) * 3;
    let bottom_left = (y1 * width + x0) * 3;
    let bottom_right = (y1 * width + x1) * 3;
    let channel = |offset, channel| {
        let value = source[offset + channel];
        f64::from(channel_map.map_or(value, |map| map[usize::from(value)]))
    };
    std::array::from_fn(|index| {
        let top = channel(top_left, index) * (1.0 - fx) + channel(top_right, index) * fx;
        let bottom = channel(bottom_left, index) * (1.0 - fx) + channel(bottom_right, index) * fx;
        (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    // Keep the original image::get_pixel implementation independent of the
    // optimized byte-offset renderer so a rounding/coordinate regression fails.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn legacy_rotation(image: &RgbImage, degrees: i32) -> RgbImage {
        match degrees.rem_euclid(360) {
            0 => image.clone(),
            90 => image::imageops::rotate90(image),
            180 => image::imageops::rotate180(image),
            270 => image::imageops::rotate270(image),
            _ => {
                let geometry = RotationGeometry::new(image.width(), image.height(), degrees);
                let (width, height) = geometry.dimensions();
                RgbImage::from_fn(width, height, |x, y| {
                    let (x, y) = geometry.source_pixel(x, y);
                    if image.width() == 0
                        || image.height() == 0
                        || x < -0.5
                        || y < -0.5
                        || x > f64::from(image.width()) - 0.5
                        || y > f64::from(image.height()) - 0.5
                    {
                        return Rgb([0; 3]);
                    }
                    let x = x.clamp(0.0, f64::from(image.width() - 1));
                    let y = y.clamp(0.0, f64::from(image.height() - 1));
                    let x0 = x.floor() as u32;
                    let y0 = y.floor() as u32;
                    let x1 = (x0 + 1).min(image.width() - 1);
                    let y1 = (y0 + 1).min(image.height() - 1);
                    let fx = x - f64::from(x0);
                    let fy = y - f64::from(y0);
                    Rgb(std::array::from_fn(|channel| {
                        let top = f64::from(image.get_pixel(x0, y0)[channel]) * (1.0 - fx)
                            + f64::from(image.get_pixel(x1, y0)[channel]) * fx;
                        let bottom = f64::from(image.get_pixel(x0, y1)[channel]) * (1.0 - fx)
                            + f64::from(image.get_pixel(x1, y1)[channel]) * fx;
                        (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
                    }))
                })
            }
        }
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)] // Deterministic test pixels wrap intentionally.
    fn direct_destination_matches_legacy_rotation_for_every_angle_and_mirror() {
        let map = std::array::from_fn(|index| (index as u8).wrapping_add(31));
        for (width, height) in [(1, 1), (1, 7), (7, 1), (17, 13)] {
            let source = RgbImage::from_fn(width, height, |x, y| {
                Rgb([
                    (x * 67 + y * 73) as u8,
                    (x * 11 + y * 149) as u8,
                    (x * 173 + y * 31) as u8,
                ])
            });
            for angle in -1..=360 {
                for mirror in [false, true] {
                    for channel_map in [None, Some(&map)] {
                        let mut adjusted = source.clone();
                        if let Some(map) = channel_map {
                            for value in adjusted.as_mut() {
                                *value = map[usize::from(*value)];
                            }
                        }
                        let mut expected = legacy_rotation(&adjusted, angle);
                        if mirror {
                            expected = image::imageops::flip_horizontal(&expected);
                        }
                        let (w, h, output) = transform_rgb_to_buffer(
                            source.as_raw(),
                            width,
                            height,
                            angle,
                            mirror,
                            channel_map,
                            |w, h| vec![0; w as usize * h as usize * 3],
                        )
                        .unwrap();
                        assert_eq!(
                            (w, h),
                            expected.dimensions(),
                            "dimensions {width}x{height}, angle={angle}, mirror={mirror}"
                        );
                        assert_eq!(
                            output,
                            *expected.as_raw(),
                            "pixels {width}x{height}, angle={angle}, mirror={mirror}, mapped={}",
                            channel_map.is_some()
                        );
                    }
                }
                assert_eq!(
                    rotate_rgb(source.clone(), angle),
                    legacy_rotation(&source, angle)
                );
            }
        }
    }

    #[test]
    fn direct_destination_rejects_invalid_buffers_before_rendering() {
        let called = std::cell::Cell::new(false);
        for (rgb, width, height) in [(&[0_u8; 3][..], 2, 1), (&[][..], 0, 0)] {
            assert!(matches!(
                transform_rgb_to_buffer(rgb, width, height, 37, false, None, |_, _| {
                    called.set(true);
                    Vec::<u8>::new()
                }),
                Err(ImagingError::InvalidBufferSize)
            ));
            assert!(!called.get());
        }
        for bytes in [0, 2, 4] {
            assert!(matches!(
                transform_rgb_to_buffer(&[0; 3], 1, 1, 0, false, None, |_, _| vec![0; bytes]),
                Err(ImagingError::InvalidBufferSize)
            ));
        }
    }

    #[test]
    fn owned_rotation_preserves_empty_image_behavior() {
        for (width, height) in [(0, 0), (0, 3), (3, 0)] {
            for angle in [0, 37, 90, 180, 270] {
                let source = RgbImage::new(width, height);
                assert_eq!(
                    rotate_rgb(source.clone(), angle),
                    legacy_rotation(&source, angle)
                );
            }
        }
    }

    #[test]
    fn precise_rotation_preserves_the_whole_non_square_image() {
        let source = RgbImage::from_pixel(320, 240, Rgb([200, 80, 40]));
        let output = rotate_rgb(source.clone(), 37);
        assert_eq!(output.dimensions(), (400, 385));
        assert_eq!(output.get_pixel(200, 192), &Rgb([200, 80, 40]));
        assert_eq!(output.get_pixel(0, 0), &Rgb([0, 0, 0]));
        for (x, y) in [(0.02, 0.02), (0.98, 0.02), (0.02, 0.98), (0.98, 0.98)] {
            let (x, y) = RotationGeometry::new(320, 240, 37).project(x, y, false);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let pixel =
                output.get_pixel((f64::from(x) * 400.0) as u32, (f64::from(y) * 385.0) as u32);
            assert_eq!(pixel, &Rgb([200, 80, 40]));
        }
        assert_eq!(rotate_rgb(source.clone(), 360), source);
    }
    #[test]
    fn cardinal_projection_and_mirror_stay_exact() {
        let g = RotationGeometry::new(320, 240, 90);
        assert_eq!(g.dimensions(), (240, 320));
        let (x, y) = g.project(0.2, 0.7, false);
        assert!((x - 0.3).abs() < 0.0001 && (y - 0.2).abs() < 0.0001);
        let (mx, my) = g.project(0.2, 0.7, true);
        assert!((mx - 0.7).abs() < 0.0001 && (my - y).abs() < 0.0001);
    }
}
