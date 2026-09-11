//! Local, read-only previews. Never hand a filename or URI to an image decoder.
use super::*;
use gtk::gdk_pixbuf::{Colorspace, Pixbuf, PixbufLoader};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::sync::{OnceLock, Weak};

pub(super) const MAX_TILES: usize = 100;
const MAX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PIXELS: i64 = 16_000_000;

pub(super) fn eligible(entry: &FileEntry) -> bool {
    entry.is_complete()
        && Path::new(entry.name())
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| {
                [
                    "jpg", "jpeg", "png", "webp", "gif", "bmp", "heic", "heif", "avif", "tif",
                    "tiff",
                ]
                .contains(&s.to_ascii_lowercase().as_str())
            })
}

#[derive(Clone)]
struct Source {
    root: PathBuf,
    relative: String,
    hash: String,
    size: u64,
}
#[derive(Debug)]
struct Pixels {
    bytes: Vec<u8>,
    width: i32,
    height: i32,
    stride: i32,
    alpha: bool,
}
struct Request {
    source: Source,
    edge: i32,
    alive: Weak<()>,
    reply: mpsc::SyncSender<Option<Pixels>>,
}

fn worker() -> &'static mpsc::SyncSender<Request> {
    static WORKER: OnceLock<mpsc::SyncSender<Request>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (send, recv) = mpsc::sync_channel::<Request>(128);
        std::thread::spawn(move || {
            for request in recv {
                if request.alive.upgrade().is_none() {
                    continue;
                }
                let result = decode(&request.source, request.edge).ok();
                let _ = request.reply.try_send(result);
            }
        });
        send
    })
}

// Some system loaders decode before emitting size-prepared. Check raster headers
// ourselves before invoking any native loader, not just inside its callback.
fn dimensions(bytes: &[u8], format: &str) -> Result<(u32, u32)> {
    if format == "png" {
        anyhow::ensure!(
            bytes.len() >= 33 && &bytes[12..16] == b"IHDR",
            "Invalid PNG header"
        );
        return Ok((
            u32::from_be_bytes(bytes[16..20].try_into()?),
            u32::from_be_bytes(bytes[20..24].try_into()?),
        ));
    }
    let mut offset = 2;
    while offset < bytes.len() {
        anyhow::ensure!(bytes[offset] == 0xff, "Invalid JPEG marker");
        while offset < bytes.len() && bytes[offset] == 0xff {
            offset += 1;
        }
        let marker = *bytes.get(offset).context("Truncated JPEG marker")?;
        offset += 1;
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let length_bytes = bytes
            .get(offset..offset + 2)
            .context("Truncated JPEG segment")?;
        let length = usize::from(u16::from_be_bytes(length_bytes.try_into()?));
        anyhow::ensure!(
            length >= 2 && offset + length <= bytes.len(),
            "Invalid JPEG segment length"
        );
        if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
            anyhow::ensure!(length >= 8, "Truncated JPEG frame");
            return Ok((
                u32::from(u16::from_be_bytes(
                    bytes[offset + 5..offset + 7].try_into()?,
                )),
                u32::from(u16::from_be_bytes(
                    bytes[offset + 3..offset + 5].try_into()?,
                )),
            ));
        }
        offset += length;
    }
    bail!("Missing JPEG dimensions")
}

fn decode(source: &Source, edge: i32) -> Result<Pixels> {
    anyhow::ensure!((1..=1024).contains(&edge), "Invalid preview size");
    anyhow::ensure!(
        source.size > 0 && source.size <= MAX_BYTES,
        "Preview size limit"
    );
    // Root opens every component with NOFOLLOW, and rejects non-regular files.
    let root = crate::directory::receiver::Root::open(&source.root)?;
    let file = root.read(&source.relative)?;
    anyhow::ensure!(file.metadata()?.len() == source.size, "File changed");
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 == source.size && hex::encode(Sha256::digest(&bytes)) == source.hash,
        "File changed"
    );
    // Explicit raster decoders only: a disguised SVG must never load resources.
    let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "jpeg"
    } else {
        bail!("Preview format not supported")
    };
    let (width, height) = dimensions(&bytes, format)?;
    anyhow::ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_PIXELS as u64,
        "Preview pixel limit"
    );
    let loader = PixbufLoader::with_type(format)?;
    let rejected = Rc::new(Cell::new(false));
    let flag = rejected.clone();
    loader.connect_size_prepared(move |loader, width, height| {
        if width <= 0 || height <= 0 || i64::from(width) * i64::from(height) > MAX_PIXELS {
            flag.set(true);
            loader.set_size(1, 1);
        } else {
            let scale = (f64::from(edge) / f64::from(width.max(height))).min(1.0);
            loader.set_size(
                (f64::from(width) * scale).round().max(1.0) as i32,
                (f64::from(height) * scale).round().max(1.0) as i32,
            );
        }
    });
    let loaded = (|| -> Result<()> {
        for chunk in bytes.chunks(4096) {
            loader.write(chunk)?;
            anyhow::ensure!(!rejected.get(), "Preview pixel limit");
        }
        Ok(())
    })();
    let closed = loader.close();
    loaded?;
    closed?;
    let pixbuf = loader.pixbuf().context("No decoded image")?;
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    anyhow::ensure!(
        pixbuf.width() <= edge && pixbuf.height() <= edge,
        "Decoder exceeded preview size"
    );
    Ok(Pixels {
        bytes: pixbuf.read_pixel_bytes().as_ref().to_vec(),
        width: pixbuf.width(),
        height: pixbuf.height(),
        stride: pixbuf.rowstride(),
        alpha: pixbuf.has_alpha(),
    })
}

fn preview(source: Option<Source>, edge: i32, height: i32) -> gtk::Stack {
    let stack = gtk::Stack::new();
    stack.set_size_request(120, height);
    let missing = gtk::Image::from_icon_name("image-missing-symbolic");
    missing.set_pixel_size(24);
    missing.set_tooltip_text(Some("Preview unavailable · original file is unchanged"));
    stack.add_named(&missing, Some("missing"));
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_keep_aspect_ratio(true);
    picture.set_alternative_text(Some("Local image preview"));
    stack.add_named(&picture, Some("image"));
    let Some(source) = source else {
        return stack;
    };
    let (send, receive) = mpsc::sync_channel(1);
    let alive = Arc::new(());
    let request = Request {
        source,
        edge,
        alive: Arc::downgrade(&alive),
        reply: send,
    };
    if worker().try_send(request).is_err() {
        return stack;
    }
    stack.add_named(&loading_ring(20), Some("loading"));
    stack.set_visible_child_name("loading");
    let weak = stack.downgrade();
    glib::timeout_add_local(Duration::from_millis(40), move || {
        let _keep_alive = &alive;
        let Some(stack) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        match receive.try_recv() {
            Ok(Some(pixels)) => {
                let pixbuf = Pixbuf::from_bytes(
                    &glib::Bytes::from_owned(pixels.bytes),
                    Colorspace::Rgb,
                    pixels.alpha,
                    8,
                    pixels.width,
                    pixels.height,
                    pixels.stride,
                );
                // Do not retain the picture from this timer: dropping the page cancels work.
                if let Some(picture) = stack.child_by_name("image").and_downcast::<gtk::Picture>() {
                    picture.set_pixbuf(Some(&pixbuf));
                }
                stack.set_visible_child_name("image");
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            _ => {
                stack.set_visible_child_name("missing");
                glib::ControlFlow::Break
            }
        }
    });
    stack
}

pub(super) fn tile(entry: &FileEntry, root: &Path, parent: &adw::ApplicationWindow) -> gtk::Button {
    let source = if let FileEntry::Saved(record) = entry {
        record
            .stored_path
            .strip_prefix(root)
            .ok()
            .and_then(|p| p.to_str())
            .map(|relative| Source {
                root: root.to_path_buf(),
                relative: relative.to_owned(),
                hash: record.sha256.clone(),
                size: record.size,
            })
    } else {
        None
    };
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.append(&preview(source.clone(), 256, 132));
    let name = gtk::Label::new(Some(entry.name()));
    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    name.set_max_width_chars(16);
    content.append(&name);
    let button = gtk::Button::builder()
        .child(&content)
        .tooltip_text(entry.name())
        .build();
    button.add_css_class("flat");
    button.update_property(&[gtk::accessible::Property::Label(&format!(
        "Preview {}",
        entry.name()
    ))]);
    let parent = parent.downgrade();
    let title = entry.name().to_owned();
    button.connect_clicked(move |_| {
        let Some(parent) = parent.upgrade() else {
            return;
        };
        let window = adw::Window::builder()
            .transient_for(&parent)
            .modal(true)
            .title(&title)
            .default_width(640)
            .default_height(480)
            .build();
        window.add_css_class("mirelay");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&adw::WindowTitle::new(
            &title,
            "Local preview · original unchanged",
        )));
        content.append(&header);
        let image = preview(source.clone(), 1024, 240);
        image.set_vexpand(true);
        content.append(&image);
        window.set_content(Some(&content));
        window.present();
    });
    button
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(bytes: &[u8]) -> (tempfile::TempDir, Source) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("image.png"), bytes).unwrap();
        let source = Source {
            root: dir.path().to_owned(),
            relative: "image.png".into(),
            hash: hex::encode(Sha256::digest(bytes)),
            size: bytes.len() as u64,
        };
        (dir, source)
    }
    const PNG: &[u8] =
        include_bytes!("../../assets/brand/MiRelay-brand-kit-v1/icons/png/mirelay-512.png");
    #[test]
    #[ignore = "requires a graphical session; exercises worker results and destroyed previews"]
    fn gtk_preview_decodes_and_survives_rapid_replacement() {
        adw::init().unwrap();
        let (dir, source) = fixture(PNG);
        let window = gtk::Window::new();
        for _ in 0..160 {
            window.set_child(Some(&preview(Some(source.clone()), 256, 132)));
        }
        window.set_child(gtk::Widget::NONE);
        let context = glib::MainContext::default();
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_millis(200) {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let image = preview(Some(source), 256, 132);
        window.set_child(Some(&image));
        window.present();
        let start = std::time::Instant::now();
        while image.visible_child_name().as_deref() == Some("loading")
            && start.elapsed() < Duration::from_secs(5)
        {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(image.visible_child_name().as_deref(), Some("image"));
        assert_eq!(std::fs::read(dir.path().join("image.png")).unwrap(), PNG);
        window.close();
    }
    #[test]
    fn bounded_decode_preserves_original() {
        let (dir, source) = fixture(PNG);
        let pixels = decode(&source, 256).unwrap();
        assert_eq!((pixels.width, pixels.height), (256, 256));
        assert!(pixels.bytes.len() <= 256 * 256 * 4);
        assert_eq!(std::fs::read(dir.path().join("image.png")).unwrap(), PNG);
        assert!(decode(&source, 0).is_err());
        assert!(decode(&source, 1025).is_err());
    }
    #[test]
    fn jpeg_is_scaled_and_large_declared_dimensions_are_rejected() {
        let pixbuf = Pixbuf::new(Colorspace::Rgb, false, 8, 600, 300).unwrap();
        pixbuf.fill(0xffffffff);
        let jpeg = pixbuf.save_to_bufferv("jpeg", &[]).unwrap();
        let (_dir, source) = fixture(&jpeg);
        let decoded = decode(&source, 256).unwrap();
        assert_eq!((decoded.width, decoded.height), (256, 128));

        // Change IHDR dimensions and its CRC, without allocating a huge bitmap.
        let mut huge = PNG.to_vec();
        huge[16..20].copy_from_slice(&5_000u32.to_be_bytes());
        huge[20..24].copy_from_slice(&5_000u32.to_be_bytes());
        let mut crc = !0u32;
        for byte in &huge[12..29] {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320u32 & 0u32.wrapping_sub(crc & 1));
            }
        }
        huge[29..33].copy_from_slice(&(!crc).to_be_bytes());
        let (_dir, source) = fixture(&huge);
        let error = decode(&source, 256).unwrap_err().to_string();
        assert!(error.contains("pixel limit"), "{error}");
    }
    #[test]
    fn corrupt_unsupported_changed_missing_and_oversized_are_rejected() {
        for bytes in [
            b"garbage".as_slice(),
            b"<svg xmlns='http://www.w3.org/2000/svg'/>".as_slice(),
            &PNG[..32],
        ] {
            let (_dir, source) = fixture(bytes);
            assert!(decode(&source, 256).is_err());
        }
        let (dir, mut source) = fixture(PNG);
        source.hash = "0".repeat(64);
        assert!(decode(&source, 256).is_err());
        source.size = MAX_BYTES + 1;
        assert!(decode(&source, 256).is_err());
        std::fs::remove_file(dir.path().join("image.png")).unwrap();
        source.size = PNG.len() as u64;
        assert!(decode(&source, 256).is_err());
    }
    #[test]
    fn symlinks_and_escape_paths_are_rejected() {
        let (dir, mut source) = fixture(PNG);
        std::os::unix::fs::symlink(dir.path().join("image.png"), dir.path().join("link.png"))
            .unwrap();
        source.relative = "link.png".into();
        assert!(decode(&source, 256).is_err());
        source.relative = "../image.png".into();
        assert!(decode(&source, 256).is_err());
        std::os::unix::fs::symlink(dir.path(), dir.path().join("nested")).unwrap();
        source.relative = "nested/image.png".into();
        assert!(decode(&source, 256).is_err());
    }

    #[test]
    fn header_parser_rejects_truncation_zero_lengths_and_missing_frames() {
        for bytes in [
            vec![],
            vec![0xff, 0xd8],
            vec![0xff, 0xd8, 0xff],
            vec![0xff, 0xd8, 0xff, 0xe0, 0, 0],
            vec![0xff, 0xd8, 0xff, 0xc0, 0, 2],
            vec![0xff, 0xd8, 0xff, 0xda, 0, 2],
        ] {
            assert!(dimensions(&bytes, "jpeg").is_err());
        }
        for size in 0..33 {
            assert!(dimensions(&PNG[..size], "png").is_err());
        }
        assert_eq!(dimensions(PNG, "png").unwrap(), (512, 512));
    }
}
