//! Model input is prepared once, away from the session and Tokio workers.
use super::*;
use base64::{Engine, prelude::BASE64_STANDARD};
use std::io::{Cursor, Read};
use tokio::sync::Semaphore;

const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_PIXELS: u64 = 40_000_000;
const MAX_EDGE: u32 = 2000;
const MAX_JPEG_BYTES: usize = 1024 * 1024;
const MAX_IMAGES: usize = 4;
// The permit is held by the blocking worker, even if its caller is stopped.
static DECODERS: Semaphore = Semaphore::const_new(2);

pub(super) async fn tool_output(output: &ToolOutput, stop: &Stop) -> ToolOutput {
    let mut content = output.as_content().to_vec();
    let mut size_notes = Vec::new();
    for (index, part) in content.iter_mut().enumerate() {
        let ToolResultContent::Image(image) = part else {
            continue;
        };
        let DocumentSourceKind::Base64(data) = &image.data else {
            *part = ToolResultContent::text(
                "Tool image URL was not fetched; image bytes must be validated before replay.",
            );
            continue;
        };
        if data.len() as u64 > (MAX_FILE_BYTES * 4 / 3 + 4) {
            *part = ToolResultContent::text("Tool image exceeds the input byte budget.");
            continue;
        }
        let data = data.clone();
        let permit = tokio::select! {
            biased;
            () = stop.raised() => { *part = ToolResultContent::text("Tool image preparation cancelled."); continue; }
            permit = DECODERS.acquire() => permit.expect("decoder semaphore stays open"),
        };
        let work = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let bytes = BASE64_STANDARD
                .decode(data)
                .map_err(|_| "Invalid tool image encoding")?;
            let source = image::ImageReader::new(Cursor::new(&bytes))
                .with_guessed_format()
                .map_err(|_| "Invalid tool image")?
                .into_dimensions()
                .map_err(|_| "Invalid tool image")?;
            let bytes = normalize(&bytes)?;
            let shown = image::ImageReader::new(Cursor::new(&bytes))
                .with_guessed_format()
                .map_err(|_| "Invalid normalized image")?
                .into_dimensions()
                .map_err(|_| "Invalid normalized image")?;
            Ok((bytes, source, shown))
        });
        let result = tokio::select! {
            biased;
            () = stop.raised() => Err("Tool image preparation cancelled"),
            result = work => result.unwrap_or(Err("Tool image preparation failed")),
        };
        *part = match result {
            Ok((bytes, source, shown)) => {
                if source != shown {
                    size_notes.push(ToolResultContent::text(format!(
                        "Tool output block {} was resized from {}×{} to {}×{}. Map image coordinates back to the original dimensions before using a coordinate tool.",
                        index + 1, source.0, source.1, shown.0, shown.1,
                    )));
                }
                ToolResultContent::Image(Image {
                    data: DocumentSourceKind::Base64(BASE64_STANDARD.encode(bytes)),
                    media_type: Some(ImageMediaType::JPEG),
                    detail: None,
                    additional_params: None,
                })
            }
            Err(reason) => ToolResultContent::text(reason),
        };
    }
    content.extend(size_notes);
    ToolOutput::content(content).expect("replacing image blocks preserves nonempty output")
}

pub(super) fn limit_history(history: &mut [Message], model: &str) {
    let mut remaining = if model.starts_with("groq/") {
        3
    } else {
        MAX_IMAGES
    };
    let mut bytes = 4 * MAX_JPEG_BYTES * 4 / 3 + 16;
    for message in history.iter_mut().rev() {
        if let Message::User { content } = message {
            for item in content.iter_mut().rev() {
                match item {
                    UserContent::Image(image) => {
                        let size = match &image.data {
                            DocumentSourceKind::Base64(data) => data.len(),
                            _ => bytes + 1,
                        };
                        if remaining > 0 && size <= bytes {
                            remaining -= 1;
                            bytes -= size;
                        } else {
                            *item = UserContent::text(
                                "Earlier image pixels omitted to fit the request image budget; the attachment path remains above.",
                            );
                        }
                    }
                    UserContent::ToolResult(result) => {
                        for part in result.content.iter_mut().rev() {
                            if let ToolResultContent::Image(image) = part {
                                let size = match &image.data {
                                    DocumentSourceKind::Base64(data) => data.len(),
                                    _ => bytes + 1,
                                };
                                if remaining > 0 && size <= bytes {
                                    remaining -= 1;
                                    bytes -= size;
                                } else {
                                    *part = ToolResultContent::text(
                                        "Tool image omitted to fit the request image budget.",
                                    );
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum Input {
    Prepared(Message),
    Attachments(String, Vec<Attachment>),
}

impl From<Message> for Input {
    fn from(message: Message) -> Self {
        Self::Prepared(message)
    }
}

impl Input {
    pub(super) async fn prepare(self, stop: &Stop) -> Message {
        let Self::Attachments(text, attachments) = self else {
            let Self::Prepared(message) = self else {
                unreachable!()
            };
            return message;
        };
        let fallback = || {
            Message::user(paths(
                &text,
                &attachments,
                "image preparation cancelled; on disk only",
            ))
        };
        let permit = tokio::select! {
            biased;
            () = stop.raised() => return fallback(),
            permit = DECODERS.acquire() => permit.expect("decoder semaphore stays open"),
        };
        let input_text = text.clone();
        let input_files = attachments.clone();
        let work = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            user_message(&input_text, &input_files)
        });
        tokio::select! {
            biased;
            () = stop.raised() => fallback(),
            result = work => result.unwrap_or_else(|_| Message::user(paths(&text, &attachments, "image preparation failed; on disk only"))),
        }
    }
}

fn paths(text: &str, attachments: &[Attachment], reason: &str) -> String {
    if attachments.is_empty() {
        return text.into();
    }
    format!(
        "{text}\n\nAttached files:\n{}",
        attachments
            .iter()
            .map(|a| {
                if a.kind == AttachmentKind::Image {
                    format!("{} ({reason})", a.path)
                } else {
                    a.path.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    )
}

pub(super) fn user_message(text: &str, attachments: &[Attachment]) -> Message {
    let mut images = Vec::new();
    let mut paths = Vec::new();
    for attachment in attachments {
        let result = if attachment.kind != AttachmentKind::Image {
            None
        } else if images.len() >= MAX_IMAGES {
            Some(Err("four-image budget reached; on disk only"))
        } else {
            Some(read_image(attachment))
        };
        match result {
            Some(Ok(bytes)) => {
                images.push(UserContent::Image(Image {
                    data: DocumentSourceKind::Base64(BASE64_STANDARD.encode(bytes)),
                    media_type: Some(ImageMediaType::JPEG),
                    detail: None,
                    additional_params: None,
                }));
                paths.push(attachment.path.clone());
            }
            Some(Err(reason)) => paths.push(format!("{} ({reason})", attachment.path)),
            None => paths.push(attachment.path.clone()),
        }
    }
    let text = if paths.is_empty() {
        text.into()
    } else {
        format!("{text}\n\nAttached files:\n{}", paths.join("\n"))
    };
    let mut content = vec![UserContent::text(text)];
    content.extend(images);
    Message::User { content }
}

fn read_image(attachment: &Attachment) -> Result<Vec<u8>, &'static str> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options
        .open(&attachment.path)
        .map_err(|_| "not readable; on disk only")?;
    if !file
        .metadata()
        .map_err(|_| "not readable; on disk only")?
        .is_file()
    {
        return Err("not a regular file; on disk only");
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "not readable; on disk only")?;
    normalize(&bytes)
}

pub(super) fn normalize(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("image exceeds 20 MiB; on disk only");
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "invalid image; on disk only")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16000);
    limits.max_image_height = Some(16000);
    limits.max_alloc = Some(160 * 1024 * 1024);
    reader.limits(limits);
    use image::ImageDecoder;
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| "unsupported or invalid image (convert HEIC to JPEG); on disk only")?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("image exceeds 40 megapixels; on disk only");
    }
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = image::DynamicImage::from_decoder(decoder)
        .map_err(|_| "could not decode image; on disk only")?;
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
            .map_err(|_| "could not encode image; on disk only")?;
        if out.len() <= MAX_JPEG_BYTES {
            return Ok(out);
        }
        if rgb.width().max(rgb.height()) <= 256 {
            return Err("image exceeds encoded budget; on disk only");
        }
        rgb = image::imageops::resize(
            &rgb,
            (rgb.width() * 3 / 4).max(1),
            (rgb.height() * 3 / 4).max(1),
            image::imageops::FilterType::Triangle,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(path: &Path) -> Attachment {
        Attachment {
            kind: AttachmentKind::Image,
            name: "photo.png".into(),
            path: path.display().to_string(),
            mime_type: Some("image/gif".into()),
            size: None,
        }
    }

    #[test]
    fn corrupt_images_never_enter_history_and_the_question_survives() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.png");
        fs::write(&path, b"\x89PNG\r\n\x1a\n").unwrap();
        let message = user_message("what is this?", &[attachment(&path)]);
        let Message::User { content } = message else {
            panic!()
        };
        assert_eq!(content.len(), 1);
        assert!(
            match &content[0] {
                UserContent::Text(text) => &text.text,
                _ => panic!("text expected"),
            }
            .contains("on disk only")
        );
        assert!(
            match &content[0] {
                UserContent::Text(text) => &text.text,
                _ => panic!("text expected"),
            }
            .contains("what is this?")
        );
    }

    #[test]
    fn a_twelve_megapixel_image_is_a_bounded_jpeg_regardless_of_declared_mime() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.png");
        image::RgbaImage::from_pixel(4000, 3000, image::Rgba([0, 0, 0, 0]))
            .save(&path)
            .unwrap();
        let Message::User { content } = user_message("inspect", &[attachment(&path)]) else {
            panic!()
        };
        let UserContent::Image(image) = &content[1] else {
            panic!()
        };
        assert_eq!(image.media_type, Some(ImageMediaType::JPEG));
        let DocumentSourceKind::Base64(data) = &image.data else {
            panic!()
        };
        let bytes = BASE64_STANDARD.decode(data).unwrap();
        assert!(bytes.len() <= MAX_JPEG_BYTES);
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (2000, 1500));
        assert!(
            decoded.get_pixel(100, 100)[0] > 250,
            "transparent pixels flatten to white"
        );
    }

    #[tokio::test]
    async fn cancellation_before_preparation_keeps_paths_without_reading_pixels() {
        let stop = Stop::default();
        stop.raise();
        let message =
            Input::Attachments("inspect".into(), vec![attachment(Path::new("/absent.png"))])
                .prepare(&stop)
                .await;
        let Message::User { content } = message else {
            panic!()
        };
        assert_eq!(content.len(), 1);
        assert!(
            match &content[0] {
                UserContent::Text(text) => &text.text,
                _ => panic!("text expected"),
            }
            .contains("cancelled")
        );
    }

    #[tokio::test]
    async fn stop_does_not_wait_for_an_available_decoder() {
        let permits = DECODERS.acquire_many(2).await.unwrap();
        let stop = Arc::new(Stop::default());
        let worker_stop = stop.clone();
        let work = tokio::spawn(async move {
            Input::Attachments(
                "keep this request".into(),
                vec![attachment(Path::new("/absent.jpg"))],
            )
            .prepare(&worker_stop)
            .await
        });
        tokio::task::yield_now().await;
        assert!(!work.is_finished());
        stop.raise();
        let message = tokio::time::timeout(std::time::Duration::from_secs(1), work)
            .await
            .unwrap()
            .unwrap();
        let rendered = serde_json::to_string(&message).unwrap();
        assert!(rendered.contains("keep this request"));
        assert!(rendered.contains("/absent.jpg"));
        assert!(rendered.contains("cancelled"));
        drop(permits);
    }

    #[test]
    fn the_request_budget_omits_older_images_first() {
        let image = Image {
            data: DocumentSourceKind::Base64("fixture".into()),
            media_type: Some(ImageMediaType::JPEG),
            detail: None,
            additional_params: None,
        };
        let mut history = vec![Message::User {
            content: (0..5).map(|_| UserContent::Image(image.clone())).collect(),
        }];
        limit_history(&mut history, "groq/model");
        let Message::User { content } = &history[0] else {
            panic!()
        };
        assert!(matches!(content[0], UserContent::Text(_)));
        assert!(matches!(content[1], UserContent::Text(_)));
        assert_eq!(
            content
                .iter()
                .filter(|part| matches!(part, UserContent::Image(_)))
                .count(),
            3
        );
    }
}
