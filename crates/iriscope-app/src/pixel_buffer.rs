//! Decode display images directly into Slint-owned storage.
use iriscope_imaging::{ImagingError, decode_mjpeg_to_buffer};
use slint::{Rgb8Pixel, SharedPixelBuffer};

struct DisplayPixels(SharedPixelBuffer<Rgb8Pixel>);

impl AsMut<[u8]> for DisplayPixels {
    fn as_mut(&mut self) -> &mut [u8] {
        self.0.make_mut_bytes()
    }
}

pub(crate) fn decode_mjpeg_pixels(
    jpeg: &[u8],
) -> Result<SharedPixelBuffer<Rgb8Pixel>, ImagingError> {
    decode_mjpeg_to_buffer(jpeg, |width, height| {
        DisplayPixels(SharedPixelBuffer::new(width, height))
    })
    .map(|(_, _, buffer)| buffer.0)
}
