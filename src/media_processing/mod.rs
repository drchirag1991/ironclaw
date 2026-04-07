//! Image processing middleware.
//!
//! Provides a [`MediaProcessingMiddleware`] that detects image attachments
//! on incoming messages and performs resizing and format conversion
//! to ensure compatibility and efficiency.

use image::{DynamicImage, ImageFormat};
use std::io::Cursor;
use crate::channels::{AttachmentKind, IncomingMessage};

/// Maximum image dimension (width or height) to keep (2048px).
const MAX_IMAGE_DIMENSION: u32 = 2048;

/// Middleware that processes image attachments on incoming messages.
///
/// For each image attachment with inline data, attempts to:
/// 1. Decode the image
/// 2. Resize if it exceeds MAX_IMAGE_DIMENSION while preserving aspect ratio
/// 3. Convert to JPEG
/// 4. Update the attachment's data and MIME type
#[derive(Default)]
pub struct MediaProcessingMiddleware;

impl MediaProcessingMiddleware {
    pub fn new() -> Self {
        Self
    }

    /// Process an incoming message, processing image attachments.
    pub async fn process(&self, msg: &mut IncomingMessage) {
        for attachment in msg.attachments.iter_mut() {
            if attachment.kind != AttachmentKind::Image {
                continue;
            }

            if attachment.data.is_empty() {
                continue;
            }

            let data = &attachment.data;
            
            // Try to load the image
            let img = match image::load_from_memory(data) {
                Ok(img) => img,
                Err(e) => {
                    tracing::warn!(
                        attachment_id = %attachment.id,
                        error = %e,
                        "Failed to decode image attachment, skipping processing"
                    );
                    continue;
                }
            };

            let (width, height) = (img.width(), img.height());
            let processed_img = if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
                tracing::info!(
                    attachment_id = %attachment.id,
                    width,
                    height,
                    "Resizing large image attachment"
                );
                img.resize(MAX_IMAGE_DIMENSION, MAX_IMAGE_DIMENSION, image::imageops::FilterType::Lanczos3)
            } else {
                img
            };

            // Convert to JPEG
            let mut buffer = Vec::new();
            let mut cursor = Cursor::new(&mut buffer);
            if let Err(e) = processed_img.write_to(&mut cursor, ImageFormat::Jpeg) {
                tracing::warn!(
                    attachment_id = %attachment.id,
                    error = %e,
                    "Failed to encode image as JPEG, skipping processing"
                );
                continue;
            }

            // Update attachment
            tracing::debug!(
                attachment_id = %attachment.id,
                old_size = data.len(),
                new_size = buffer.len(),
                "Image processed successfully"
            );
            attachment.data = buffer;
            attachment.mime_type = "image/jpeg".to_string();
            attachment.size_bytes = Some(attachment.data.len() as u64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::IncomingAttachment;

    fn image_attachment(mime: &str, data: Vec<u8>) -> IncomingAttachment {
        IncomingAttachment {
            id: "img_1".to_string(),
            kind: AttachmentKind::Image,
            mime_type: mime.to_string(),
            filename: Some("test.png".to_string()),
            size_bytes: Some(data.len() as u64),
            source_url: None,
            storage_key: None,
            extracted_text: None,
            data,
            duration_secs: None,
        }
    }

    #[tokio::test]
    async fn processes_png_to_jpeg() {
        // Create a small 100x100 PNG
        let mut buffer = Vec::new();
        let img = DynamicImage::new_rgb8(100, 100);
        img.write_to(&mut Cursor::new(&mut buffer), ImageFormat::Png).unwrap();

        let middleware = MediaProcessingMiddleware::new();
        let mut msg = IncomingMessage::new("test", "user1", "").with_attachments(vec![
            image_attachment("image/png", buffer),
        ]);

        middleware.process(&mut msg).await;
        assert_eq!(msg.attachments[0].mime_type, "image/jpeg");
        // Verify it can be loaded as JPEG
        let processed = image::load_from_memory(&msg.attachments[0].data).unwrap();
        assert_eq!(processed.width(), 100);
    }

    #[tokio::test]
    async fn resizes_large_image() {
        // Create a large 3000x1000 image
        let mut buffer = Vec::new();
        let img = DynamicImage::new_rgb8(3000, 1000);
        img.write_to(&mut Cursor::new(&mut buffer), ImageFormat::Png).unwrap();

        let middleware = MediaProcessingMiddleware::new();
        let mut msg = IncomingMessage::new("test", "user1", "").with_attachments(vec![
            image_attachment("image/png", buffer),
        ]);

        middleware.process(&mut msg).await;
        let processed = image::load_from_memory(&msg.attachments[0].data).unwrap();
        assert!(processed.width() <= MAX_IMAGE_DIMENSION);
        assert!(processed.height() <= MAX_IMAGE_DIMENSION);
    }
}
