use super::{Room, files::Source};
use crate::contract::Reach;
use crate::driver::CapabilityLease;
use crate::imagegen::{self, Aspect, ImageError, ImageRequest, ImageSet, ImageSettings, Reference};
use crate::sent;
use crate::spending::SpendingSettings;
use crate::tools::Workspace;
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

const REFERENCE_BYTES: u64 = 20 * 1024 * 1024;

/// An image as it came back, what it cost across every attempt, and whether
/// a subscription paid for it.
pub(crate) struct Drawn {
    pub(crate) image: imagegen::Image,
    pub(crate) spent_usd: f64,
    pub(crate) subscription: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    prompt: String,
    #[serde(default)]
    aspect: Aspect,
    #[serde(default)]
    transparent: bool,
    #[serde(default)]
    references: Vec<String>,
    style: Option<String>,
    name: Option<String>,
}

impl Room {
    pub(crate) fn spending_summary(&self) -> Result<crate::spending::SpendingSummary, String> {
        self.spending.summary()
    }

    pub(crate) async fn generate_image(
        &self,
        persona_id: &str,
        arguments: &Value,
        capability: Option<CapabilityLease>,
    ) -> Result<String, String> {
        let started = Instant::now();
        let args: Arguments = serde_json::from_value(arguments.clone()).map_err(|_| {
            "generate_image needs a prompt, a supported aspect and optional image references, style and name.".to_string()
        })?;
        if args.prompt.trim().is_empty() || args.prompt.len() > 16_000 {
            return Err("Give generate_image a prompt of 1 to 16000 bytes.".into());
        }
        if args.references.len() > 16 {
            return Err("generate_image takes at most 16 reference images.".into());
        }
        let name = image_name(args.name.as_deref(), &args.prompt)?;
        let (prompt, aspect) = imagegen::styled(args.style.as_deref(), &args.prompt, args.aspect)?;
        if self.is_quiet(persona_id) {
            return Err("This is a quiet scheduled run; make the image when you are talking with the person.".into());
        }
        let persona = self.persona(persona_id)?;
        let settings = crate::room::try_settings(self.log())?;
        let spending: SpendingSettings = serde_json::from_value(
            settings
                .get("spending")
                .cloned()
                .unwrap_or_else(|| json!({})),
        )
        .map_err(|_| "The room's spending settings could not be read.".to_string())?;
        spending.validate()?;
        let workspace = Workspace::open_with_capability(
            PathBuf::from(&persona.cwd),
            persona.reach.unwrap_or_default(),
            self.log.root().join("tool-output").join(persona_id),
            capability.clone(),
        )
        .map_err(|error| error.to_string())?;
        let references =
            tokio::task::spawn_blocking(move || read_references(&workspace, &args.references))
                .await
                .map_err(|_| "The reference images could not be read.".to_string())??;
        let request = ImageRequest {
            prompt,
            aspect,
            transparent: args.transparent,
            references,
        };
        let Drawn {
            image,
            spent_usd,
            subscription,
        } = self.draw(&request, &capability).await?;
        if let Some(capability) = &capability {
            capability.check()?;
        }
        let model = image.id.model_id;
        let transparent = image.transparent;
        let permit = crate::images::DECODERS
            .acquire()
            .await
            .map_err(|_| "The generated image could not be prepared.".to_string())?;
        let file = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            sent::generated::prepare(&name, image.bytes)
        })
        .await
        .map_err(|_| "The generated image could not be prepared.".to_string())??;
        let output = Workspace::open_with_capability(
            PathBuf::from(&persona.cwd),
            Reach::Workspace,
            self.log.root().join("tool-output").join(persona_id),
            capability.clone(),
        )
        .map_err(|error| error.to_string())?;
        let file_name = file.name.clone();
        let path =
            tokio::task::spawn_blocking(move || output.create_file_path(&file.name, &file.bytes))
                .await
                .map_err(|_| "The generated image could not be written.".to_string())?
                .map_err(|error| error.to_string())?;
        self.send_file(
            persona_id,
            Source::GeneratedImage(file_name),
            "",
            capability,
        )
        .await?;
        let mut result = json!({
            "path": path,
            "model": model,
            "costUsd": spent_usd,
            "seconds": started.elapsed().as_secs_f64(),
            "transparent": transparent,
        });
        if subscription {
            result["costUsd"] = Value::Null;
            result["billing"] = json!("subscription");
        }
        Ok(result.to_string())
    }

    /// One image from the room's image providers, the owner's pick first and
    /// then the fallback, each paid attempt within the spending cap. A
    /// subscription that fails never falls back to a paid provider.
    pub(crate) async fn draw(
        &self,
        request: &ImageRequest,
        capability: &Option<CapabilityLease>,
    ) -> Result<Drawn, String> {
        let generators = self.image_generators()?;
        let mut spent_usd = 0.0;
        let mut failure = None;
        let mut on_plan = false;
        for generator in std::iter::once(generators.primary).chain(generators.fallback) {
            let subscription = generator.subscription();
            if on_plan && !subscription {
                break;
            }
            on_plan = subscription;
            if let Some(capability) = &capability {
                capability.check()?;
            }
            if request.references.len() > generator.max_references() {
                failure = Some(
                    imagegen::ImageError::TooManyReferences {
                        max: generator.max_references(),
                    }
                    .to_string(),
                );
                continue;
            }
            let settings = crate::room::try_settings(self.log())?;
            let current: SpendingSettings = serde_json::from_value(
                settings
                    .get("spending")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            )
            .map_err(|_| "The room's spending settings could not be read.".to_string())?;
            current.validate()?;
            let estimate = generator.estimate_usd(request);
            let ledger = self.spending.clone();
            let reservation = if subscription {
                None
            } else {
                Some(
                    tokio::task::spawn_blocking(move || ledger.reserve(&current, estimate))
                        .await
                        .map_err(|_| "The image's spending could not be reserved.".to_string())??,
                )
            };
            if let Some(capability) = &capability {
                capability.check()?;
            }
            let before_send = || {
                if let Some(capability) = &capability {
                    capability.check().map_err(|_| ImageError::Revoked)?;
                }
                Ok(())
            };
            let result = generator.generate_checked(request, &before_send).await;
            let charge = match &result {
                Ok(image) => Some(image.cost_usd.unwrap_or(estimate)),
                Err(
                    ImageError::Refused {
                        status: 400..=499, ..
                    }
                    | ImageError::Revoked,
                ) => Some(0.0),
                Err(_) => None,
            };
            if let (Some(cost), Some(reservation)) = (charge, reservation) {
                tokio::task::spawn_blocking(move || reservation.charge(cost))
                    .await
                    .map_err(|_| "The image's cost could not be recorded.".to_string())??;
            }
            spent_usd += charge.unwrap_or(estimate);
            match result {
                Ok(image) => {
                    return Ok(Drawn {
                        image,
                        spent_usd,
                        subscription,
                    });
                }
                Err(error) => {
                    failure = Some(error.to_string());
                }
            }
        }
        Err(failure.unwrap_or_else(|| "No connected provider could make this image.".into()))
    }

    /// The providers the room's image settings name, the owner's pick first.
    pub(crate) fn image_generators(&self) -> Result<ImageSet, String> {
        let settings = crate::room::try_settings(self.log())?;
        let images: ImageSettings = serde_json::from_value(crate::room::normalize_setting(
            "images",
            settings.get("images").unwrap_or(&json!({})),
        )?)
        .map_err(|_| "The room's image settings could not be read.".to_string())?;
        self.resolve_images(&images)
    }

    fn resolve_images(&self, settings: &ImageSettings) -> Result<ImageSet, String> {
        #[cfg(test)]
        if let Some(generators) = super::lock(&self.image_generators).clone() {
            return Ok(generators);
        }
        let vault = self
            .vault
            .as_ref()
            .ok_or_else(|| imagegen::NOTHING_DRAWS.to_string())?;
        imagegen::resolve(vault, settings)
    }

    #[cfg(test)]
    pub(crate) fn set_image_generators(&self, generators: ImageSet) {
        *super::lock(&self.image_generators) = Some(generators);
    }
}

fn image_name(name: Option<&str>, prompt: &str) -> Result<String, String> {
    if let Some(name) = name {
        let name = name.trim();
        if name.is_empty()
            || name.len() > 100
            || name == "."
            || name == ".."
            || name
                .chars()
                .any(|character| character.is_control() || "/\\<>:\"|?*".contains(character))
        {
            return Err("Name the image with a file name, not a path (at most 100 bytes).".into());
        }
        return Ok(name.to_string());
    }
    let words: Vec<&str> = prompt
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(6)
        .collect();
    let stem = words.join("-").to_ascii_lowercase();
    let stem = if stem.is_empty() {
        "image"
    } else {
        &stem[..stem.len().min(70)]
    };
    Ok(format!(
        "{stem}-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ))
}

fn read_references(workspace: &Workspace, paths: &[String]) -> Result<Vec<Reference>, String> {
    let mut references = Vec::with_capacity(paths.len());
    for requested in paths {
        let bytes = read_workspace_image(workspace, requested, "A reference image")?;
        let format = image::guess_format(&bytes)
            .map_err(|_| "Reference images must be PNG, JPEG or WebP.".to_string())?;
        if !matches!(
            format,
            image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP
        ) {
            return Err("Reference images must be PNG, JPEG or WebP.".into());
        }
        references.push(Reference {
            mime: format.to_mime_type().into(),
            bytes,
        });
    }
    Ok(references)
}

/// The bytes of a file the teammate's workspace tools may read, at most 20 MB.
/// `noun` opens the sentences that refuse it, like "A reference image".
pub(super) fn read_workspace_image(
    workspace: &Workspace,
    requested: &str,
    noun: &str,
) -> Result<Vec<u8>, String> {
    let path = Path::new(requested);
    let requested = if workspace.reach() == Reach::Workspace && path.is_absolute() {
        // The same folder can be spelled two ways (macOS's `/var` is
        // `/private/var`), so a path that doesn't start with the root as
        // written is tried again with both resolved. The workspace still
        // confines what the relative path can open.
        let inside = path
            .strip_prefix(workspace.display_root())
            .map(Path::to_path_buf)
            .or_else(|_| {
                let root = std::fs::canonicalize(workspace.display_root()).map_err(|_| ())?;
                let real = std::fs::canonicalize(path).map_err(|_| ())?;
                real.strip_prefix(root)
                    .map(Path::to_path_buf)
                    .map_err(|_| ())
            });
        inside
            .map_err(|_| format!("{noun} must be inside your workspace."))?
            .to_string_lossy()
            .into_owned()
    } else {
        requested.to_string()
    };
    let (file, size, _) = workspace
        .open_to_send(&requested)
        .map_err(|error| error.to_string())?;
    if size > REFERENCE_BYTES {
        return Err(format!("{noun} can be at most 20 MB."));
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(REFERENCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("{noun} could not be read."))?;
    if bytes.len() as u64 > REFERENCE_BYTES {
        return Err(format!("{noun} can be at most 20 MB."));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
