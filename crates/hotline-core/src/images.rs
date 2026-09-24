//! What an image is made into before anyone else sees it (BRO-2).
//!
//! One policy for both directions. An image the person attaches is prepared
//! this way before a model is handed it, and an image a teammate sends the
//! person is prepared this way before it is stored and shown: turned upright,
//! flattened onto white, at most 2000 px on its longer edge, and a JPEG of at
//! most 1 MiB. That is inside every provider's limits, quick on a phone's
//! connection, and still sharp enough to read a screenshot.

use std::io::Cursor;
use tokio::sync::Semaphore;

/// The most an image may weigh before it is decoded at all.
pub(crate) const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_PIXELS: u64 = 40_000_000;
const MAX_EDGE: u32 = 2000;
pub(crate) const MAX_JPEG_BYTES: usize = 1024 * 1024;

/// Decoding takes CPU and memory, so at most two run at once, whoever the
/// image is for. The permit is held by the blocking worker, even if its
/// caller is stopped.
pub(crate) static DECODERS: Semaphore = Semaphore::const_new(2);

/// Why an image could not be prepared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unfit {
    TooLarge,
    Invalid,
    Unsupported,
    TooManyPixels,
    Undecodable,
    Unencodable,
    OverBudget,
}

impl Unfit {
    /// What a model reads beside the path it can still open.
    pub(crate) fn for_model(self) -> &'static str {
        match self {
            Self::TooLarge => "image exceeds 20 MiB; on disk only",
            Self::Invalid => "invalid image; on disk only",
            Self::Unsupported => {
                "unsupported or invalid image (convert HEIC to JPEG); on disk only"
            }
            Self::TooManyPixels => "image exceeds 40 megapixels; on disk only",
            Self::Undecodable => "could not decode image; on disk only",
            Self::Unencodable => "could not encode image; on disk only",
            Self::OverBudget => "image exceeds encoded budget; on disk only",
        }
    }

    /// Why a picture went to the person as a plain file, for the teammate
    /// that sent it.
    pub(crate) fn plainly(self) -> &'static str {
        match self {
            Self::TooLarge => "it is over the 20 MB a picture may be",
            Self::TooManyPixels => "it is over the 40 megapixels a picture may be",
            Self::Invalid | Self::Unsupported | Self::Undecodable => {
                "the picture in it could not be read"
            }
            Self::Unencodable | Self::OverBudget => "it could not be made small enough to show",
        }
    }
}

/// Whether these bytes are an image this policy reads: PNG, JPEG, GIF or
/// WebP, by their first bytes rather than by any name.
pub(crate) fn readable(bytes: &[u8]) -> bool {
    use image::ImageFormat;
    matches!(
        image::guess_format(bytes),
        Ok(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP)
    )
}

/// Width and height, read from the header alone.
pub(crate) fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// The image as the policy wants it: an upright JPEG within the limits.
pub(crate) fn normalize(bytes: &[u8]) -> Result<Vec<u8>, Unfit> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(Unfit::TooLarge);
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| Unfit::Invalid)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16000);
    limits.max_image_height = Some(16000);
    limits.max_alloc = Some(160 * 1024 * 1024);
    reader.limits(limits);
    use image::ImageDecoder;
    let mut decoder = reader.into_decoder().map_err(|_| Unfit::Unsupported)?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(Unfit::TooManyPixels);
    }
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = image::DynamicImage::from_decoder(decoder).map_err(|_| Unfit::Undecodable)?;
    decoded.apply_orientation(orientation);
    let resized = if decoded.width().max(decoded.height()) > MAX_EDGE {
        decoded.resize(MAX_EDGE, MAX_EDGE, image::imageops::FilterType::Triangle)
    } else {
        decoded
    };
    // Flatten transparency onto white, so transparent screenshots keep legible text.
    let rgba = resized.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (from, to) in rgba.pixels().zip(rgb.pixels_mut()) {
        let alpha = u32::from(from[3]);
        for channel in 0..3 {
            to[channel] =
                ((u32::from(from[channel]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        }
    }
    loop {
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
            .encode_image(&rgb)
            .map_err(|_| Unfit::Unencodable)?;
        if out.len() <= MAX_JPEG_BYTES {
            return Ok(out);
        }
        if rgb.width().max(rgb.height()) <= 256 {
            return Err(Unfit::OverBudget);
        }
        rgb = image::imageops::resize(
            &rgb,
            (rgb.width() * 3 / 4).max(1),
            (rgb.height() * 3 / 4).max(1),
            image::imageops::FilterType::Triangle,
        );
    }
}
