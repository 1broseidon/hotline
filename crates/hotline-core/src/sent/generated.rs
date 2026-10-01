use super::{MAX_BYTES, Prepared, clean_name, too_large};
use crate::contract::AttachmentKind;
use image::{ImageDecoder, ImageFormat};
use std::io::Cursor;

const MAX_EDGE: u32 = 2048;
const INVALID: &str = "The provider returned an image that could not be read.";

pub(crate) fn prepare(name: &str, bytes: Vec<u8>) -> Result<Prepared, String> {
    let name = clean_name(name);
    if bytes.len() as u64 > MAX_BYTES {
        return Err(too_large(&name, Some(bytes.len() as u64)));
    }
    let (bytes, mime, extension, dimensions) = match image::guess_format(&bytes) {
        Ok(format @ (ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)) => {
            let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(16000);
            limits.max_image_height = Some(16000);
            limits.max_alloc = Some(160 * 1024 * 1024);
            reader.limits(limits);
            let decoder = reader.into_decoder().map_err(|_| INVALID.to_string())?;
            let dimensions = decoder.dimensions();
            if decoder.total_bytes() > 160 * 1024 * 1024 {
                return Err("The provider returned an image too large to decode safely.".into());
            }
            if u64::from(dimensions.0) * u64::from(dimensions.1) > 40_000_000 {
                return Err("The provider returned an image over 40 megapixels.".into());
            }
            let decoded =
                image::DynamicImage::from_decoder(decoder).map_err(|_| INVALID.to_string())?;
            let (bytes, dimensions) = if dimensions.0.max(dimensions.1) > MAX_EDGE {
                let resized =
                    decoded.resize(MAX_EDGE, MAX_EDGE, image::imageops::FilterType::Triangle);
                let dimensions = (resized.width(), resized.height());
                let mut encoded = Cursor::new(Vec::new());
                resized.write_to(&mut encoded, format).map_err(|_| {
                    "The generated image could not be resized in its own format.".to_string()
                })?;
                (encoded.into_inner(), dimensions)
            } else {
                (bytes, dimensions)
            };
            let extension = match format {
                ImageFormat::Png => "png",
                ImageFormat::Jpeg => "jpg",
                ImageFormat::WebP => "webp",
                _ => unreachable!(),
            };
            (bytes, format.to_mime_type(), extension, dimensions)
        }
        _ => {
            let (bytes, dimensions) = svg(bytes)?;
            (bytes, "image/svg+xml", "svg", dimensions)
        }
    };
    if bytes.len() as u64 > MAX_BYTES {
        return Err(too_large(&name, Some(bytes.len() as u64)));
    }
    let stem = std::path::Path::new(&name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("image");
    Ok(Prepared {
        name: format!("{stem}.{extension}"),
        mime_type: mime.into(),
        kind: AttachmentKind::Image,
        bytes,
        dimensions: Some(dimensions),
        unfit: None,
    })
}

fn svg(bytes: Vec<u8>) -> Result<(Vec<u8>, (u32, u32)), String> {
    let text = std::str::from_utf8(&bytes).map_err(|_| INVALID.to_string())?;
    let document = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            nodes_limit: 100_000,
            ..Default::default()
        },
    )
    .map_err(|_| INVALID.to_string())?;
    let root = document.root_element();
    if root.tag_name().name() != "svg"
        || root
            .tag_name()
            .namespace()
            .is_some_and(|namespace| namespace != "http://www.w3.org/2000/svg")
    {
        return Err(INVALID.into());
    }
    let view_box = root.attribute("viewBox").and_then(|value| {
        let numbers: Vec<f64> = value
            .split(|character: char| character.is_whitespace() || character == ',')
            .filter(|part| !part.is_empty())
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        (numbers.len() == 4 && numbers.iter().all(|number| number.is_finite()))
            .then(|| (numbers[2], numbers[3]))
    });
    let length = |key| {
        root.attribute(key)
            .and_then(|value| value.trim().trim_end_matches("px").parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
    };
    let (width, height) = match (length("width"), length("height"), view_box) {
        (Some(width), Some(height), _) => (width, height),
        (_, _, Some((width, height))) if width > 0.0 && height > 0.0 => (width, height),
        _ => return Err("The generated SVG needs a width and height or a viewBox.".into()),
    };
    let scale = (f64::from(MAX_EDGE) / width.max(height)).min(1.0);
    let dimensions = (
        (width * scale).round().clamp(1.0, f64::from(MAX_EDGE)) as u32,
        (height * scale).round().clamp(1.0, f64::from(MAX_EDGE)) as u32,
    );
    let viewport_style = format!(
        "width:{}px!important;height:{}px!important",
        dimensions.0, dimensions.1
    );
    if length("width") == Some(f64::from(dimensions.0))
        && length("height") == Some(f64::from(dimensions.1))
        && root.attribute("viewBox").is_some()
        && root
            .attribute("style")
            .is_some_and(|style| style.ends_with(&viewport_style))
    {
        drop(document);
        return Ok((bytes, dimensions));
    }
    let start = root.range().start + 1;
    let name_end = start
        + text[start..]
            .find(|character: char| {
                character.is_whitespace() || character == '/' || character == '>'
            })
            .ok_or_else(|| INVALID.to_string())?;
    let style = root
        .attribute("style")
        .unwrap_or("")
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
    let mut attributes = format!(
        " width=\"{}\" height=\"{}\" style=\"{style};{viewport_style}\"",
        dimensions.0, dimensions.1
    );
    if root.attribute("viewBox").is_none() {
        attributes.push_str(&format!(" viewBox=\"0 0 {width} {height}\""));
    }
    let mut edits = vec![(name_end..name_end, attributes)];
    for attribute in root.attributes() {
        if attribute.namespace().is_none()
            && matches!(attribute.name(), "width" | "height" | "style")
        {
            edits.push((attribute.range(), String::new()));
        }
    }
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut resized = text.to_string();
    for (range, replacement) in edits {
        resized.replace_range(range, &replacement);
    }
    Ok((resized.into_bytes(), dimensions))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn picture(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([20, 40, 60, 0]));
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut encoded, format)
            .unwrap();
        encoded.into_inner()
    }

    #[test]
    fn generated_png_and_webp_keep_their_alpha_and_format_when_shrunk() {
        for format in [ImageFormat::Png, ImageFormat::WebP] {
            let prepared = prepare("hero.jpg", picture(format, 2500, 1250)).unwrap();
            assert_eq!(prepared.mime_type, format.to_mime_type());
            assert_eq!(prepared.dimensions, Some((2048, 1024)));
            let image = image::load_from_memory(&prepared.bytes).unwrap().to_rgba8();
            assert_eq!(image.get_pixel(0, 0)[3], 0);
        }
    }

    #[test]
    fn a_small_generated_picture_keeps_its_original_bytes() {
        let bytes = picture(ImageFormat::Png, 32, 32);
        assert_eq!(prepare("avatar.webp", bytes.clone()).unwrap().bytes, bytes);
    }

    #[test]
    fn svg_keeps_its_content_and_caps_its_viewport_without_rasterizing() {
        let bytes = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4096" height="2048" style="width:4096px"><path d="M0 0h10"/></svg>"#.to_vec();
        let prepared = prepare("diagram.png", bytes).unwrap();
        assert_eq!(prepared.mime_type, "image/svg+xml");
        assert_eq!(prepared.dimensions, Some((2048, 1024)));
        let text = std::str::from_utf8(&prepared.bytes).unwrap();
        let document = roxmltree::Document::parse(text).unwrap();
        assert_eq!(
            document.root_element().attribute("viewBox"),
            Some("0 0 4096 2048")
        );
        assert!(text.contains("<path d=\"M0 0h10\"/>"));
        assert!(text.contains("width:2048px!important"));
        assert_eq!(
            prepare("diagram.svg", prepared.bytes.clone())
                .unwrap()
                .bytes,
            prepared.bytes
        );
    }

    #[test]
    fn non_images_and_unbounded_svg_are_refused() {
        for bytes in [
            b"not an image".as_slice(),
            b"<svg/>".as_slice(),
            b"<svg width='NaN' height='20'/>".as_slice(),
        ] {
            assert!(prepare("image.png", bytes.to_vec()).is_err());
        }
    }

    #[test]
    fn a_high_bit_depth_image_cannot_bypass_the_decode_allocation_cap() {
        let pixels = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::new(1, 1);
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba16(pixels)
            .write_to(&mut encoded, ImageFormat::Png)
            .unwrap();
        let mut bytes = encoded.into_inner();
        bytes[16..20].copy_from_slice(&5000_u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&5000_u32.to_be_bytes());
        let checksum = bytes[12..29].iter().fold(u32::MAX, |mut checksum, byte| {
            checksum ^= u32::from(*byte);
            for _ in 0..8 {
                checksum = if checksum & 1 == 0 {
                    checksum >> 1
                } else {
                    (checksum >> 1) ^ 0xedb8_8320
                };
            }
            checksum
        }) ^ u32::MAX;
        bytes[29..33].copy_from_slice(&checksum.to_be_bytes());
        let error = prepare("huge.png", bytes).err().unwrap();
        assert!(error.contains("too large to decode safely"), "{error}");
    }
}
