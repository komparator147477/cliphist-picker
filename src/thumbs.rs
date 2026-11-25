//! Decoding entries for thumbnails and the preview pane. These functions run
//! on gio's blocking thread pool; their results are Send.

use crate::history;
use anyhow::Result;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::{gdk, gio, glib};

/// Max characters of text shown in the preview pane.
const MAX_TEXT_CHARS: usize = 20_000;

/// Decode an image entry and scale it to fit `w`×`h`, keeping aspect ratio.
pub fn load_image(line: &str, w: i32, h: i32) -> Result<gdk::Texture> {
    let bytes = history::decode(line)?;
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes));
    let pixbuf = Pixbuf::from_stream_at_scale(&stream, w, h, true, gio::Cancellable::NONE)?;
    #[allow(deprecated)]
    Ok(gdk::Texture::for_pixbuf(&pixbuf))
}

/// Decoded pixels; unlike `Pixbuf` they can leave the loader thread.
pub struct RawImage {
    data: Vec<u8>,
    width: i32,
    height: i32,
    rowstride: i32,
    has_alpha: bool,
}

impl RawImage {
    pub fn into_pixbuf(self) -> Pixbuf {
        Pixbuf::from_bytes(
            &glib::Bytes::from_owned(self.data),
            gtk::gdk_pixbuf::Colorspace::Rgb,
            self.has_alpha,
            8,
            self.width,
            self.height,
            self.rowstride,
        )
    }
}

/// Decode an image entry for a list thumbnail, scaled to fit `w`×`h`.
pub fn load_thumb(line: &str, w: i32, h: i32) -> Result<RawImage> {
    let bytes = history::decode(line)?;
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes));
    let pixbuf = Pixbuf::from_stream_at_scale(&stream, w, h, true, gio::Cancellable::NONE)?;
    Ok(RawImage {
        data: pixbuf.read_pixel_bytes().to_vec(),
        width: pixbuf.width(),
        height: pixbuf.height(),
        rowstride: pixbuf.rowstride(),
        has_alpha: pixbuf.has_alpha(),
    })
}

pub fn load_text(line: &str) -> Result<String> {
    let bytes = history::decode(line)?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(match text.char_indices().nth(MAX_TEXT_CHARS) {
        Some((cut, _)) => format!("{}\n…", &text[..cut]),
        None => text.into_owned(),
    })
}
