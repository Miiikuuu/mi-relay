//! Local, read-only previews. Never hand a filename or URI to an image decoder.
use super::*;
use gtk::gdk_pixbuf::{Colorspace, Pixbuf, PixbufLoader};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::sync::{OnceLock, Weak};

mod grid;
#[cfg(test)]
mod grid_tests;
mod viewer;
pub(super) use grid::AlbumGrid;

#[derive(Clone)]
pub(super) struct Layout {
    pub page: gtk::Box,
    pub clamp: adw::Clamp,
    pub details: gtk::MenuButton,
    pub activity: gtk::Expander,
    pub activity_scroll: gtk::ScrolledWindow,
    pub files: gtk::Box,
    pub title: gtk::Label,
    pub enabled: Rc<Cell<bool>>,
}

impl Layout {
    pub fn apply(&self, widgets: &Widgets, photos: bool) {
        if self.enabled.replace(photos) != photos {
            if photos {
                self.clamp.set_child(gtk::Widget::NONE);
                widgets.content_stack.add_named(&self.page, Some("album"));
                self.files.remove(&widgets.delivery_list);
                self.activity_scroll.set_child(Some(&widgets.delivery_list));
                self.page.add_css_class("album-page");
            } else {
                widgets.content_stack.remove(&self.page);
                self.clamp.set_child(Some(&self.page));
                self.activity_scroll.set_child(gtk::Widget::NONE);
                self.files.prepend(&widgets.delivery_list);
                self.page.remove_css_class("album-page");
                self.activity.set_expanded(false);
            }
        }
        self.details.set_visible(true);
        self.activity
            .set_visible(photos && widgets.delivery_list.first_child().is_some());
        widgets.photo_grid.widget.set_visible(photos);
        widgets.bridge_path.set_visible(!photos);
        widgets.activity_stack.set_vexpand(photos);
        self.files.set_vexpand(photos);
        self.title
            .set_label(if photos { "Photos" } else { "Files" });
        widgets
            .content_stack
            .set_visible_child_name(if photos { "album" } else { "bridge" });
    }
}

const MAX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PIXELS: i64 = 16_000_000;
const CACHE_BYTES: usize = 24 * 1024 * 1024;
#[cfg(test)]
static PREVIEW_POLLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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

#[derive(Clone, Debug, PartialEq, Eq)]
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
struct SharedPixels(Arc<Pixels>);
impl AsRef<[u8]> for SharedPixels {
    fn as_ref(&self) -> &[u8] {
        &self.0.bytes
    }
}
struct Request {
    source: Source,
    edge: i32,
    alive: Weak<()>,
    reply: mpsc::SyncSender<Option<Arc<Pixels>>>,
}

#[derive(PartialEq, Eq)]
struct CacheKey {
    source: Source,
    edge: i32,
    stamp: [i64; 6],
}

fn stamp(source: &Source) -> Result<[i64; 6]> {
    let root = crate::directory::receiver::Root::open(&source.root)?;
    let meta = root.read(&source.relative)?.metadata()?;
    anyhow::ensure!(meta.len() == source.size, "File changed");
    Ok([
        meta.dev() as i64,
        meta.ino() as i64,
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec(),
    ])
}

#[derive(Default)]
struct PreviewCache {
    entries: VecDeque<(CacheKey, Arc<Pixels>)>,
    bytes: usize,
}

impl PreviewCache {
    fn load(&mut self, source: &Source, edge: i32) -> Result<Arc<Pixels>> {
        // Reopen through NOFOLLOW even on a cache hit. Nanosecond change time
        // catches same-size rewrites and restored modification timestamps.
        let key = CacheKey {
            source: source.clone(),
            edge,
            stamp: stamp(source)?,
        };
        if let Some(index) = self.entries.iter().position(|(old, _)| *old == key) {
            let entry = self.entries.remove(index).unwrap();
            let pixels = entry.1.clone();
            self.entries.push_back(entry);
            return Ok(pixels);
        }
        let pixels = Arc::new(decode(source, edge)?);
        anyhow::ensure!(stamp(source)? == key.stamp, "File changed while decoding");
        self.bytes += pixels.bytes.len();
        self.entries.push_back((key, pixels.clone()));
        while self.bytes > CACHE_BYTES {
            if let Some((_, old)) = self.entries.pop_front() {
                self.bytes -= old.bytes.len();
            }
        }
        Ok(pixels)
    }
}

fn worker() -> &'static mpsc::SyncSender<Request> {
    static WORKER: OnceLock<mpsc::SyncSender<Request>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (send, recv) = mpsc::sync_channel::<Request>(128);
        std::thread::spawn(move || {
            let mut cache = PreviewCache::default();
            for request in recv {
                if request.alive.upgrade().is_none() {
                    continue;
                }
                let result = cache.load(&request.source, request.edge).ok();
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
    preview_image(source, edge, height, false)
}

fn preview_image(source: Option<Source>, edge: i32, height: i32, crop: bool) -> gtk::Stack {
    let preview = Preview::new(edge, height, crop);
    preview.bind(source);
    preview.stack
}

struct Preview {
    stack: gtk::Stack,
    source: Rc<RefCell<Option<Source>>>,
    job: Rc<RefCell<Option<glib::SourceId>>>,
    edge: i32,
    crop: bool,
}

impl Preview {
    fn new(edge: i32, height: i32, crop: bool) -> Self {
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
        if crop {
            // A quiet placeholder for a wall of photos. Transfer rows and the
            // single-image viewer retain DashRing; a grid need not animate dozens
            // of concurrent loading indicators while recycling cells.
            let placeholder = gtk::Image::from_icon_name("image-x-generic-symbolic");
            placeholder.set_pixel_size(20);
            placeholder.add_css_class("dim-label");
            stack.add_named(&placeholder, Some("loading"));
        } else {
            stack.add_named(&loading_ring(20), Some("loading"));
        }
        let source = Rc::new(RefCell::new(None::<Source>));
        let current = source.clone();
        let job = Rc::new(RefCell::new(None::<glib::SourceId>));
        let pending = job.clone();
        stack.connect_map(move |stack| {
            // End the pending RefCell borrow before installing the new job.
            let waiting = stack.visible_child_name().as_deref() == Some("loading")
                && pending.borrow().is_none();
            if waiting && let Some(source) = current.borrow().clone() {
                let id = begin_preview(stack, source, edge, crop, pending.clone());
                pending.replace(Some(id));
            }
        });
        let stopped = job.clone();
        stack.connect_unmap(move |_| {
            if let Some(id) = stopped.borrow_mut().take() {
                id.remove();
            }
        });
        Self {
            stack,
            source,
            job,
            edge,
            crop,
        }
    }

    fn bind(&self, source: Option<Source>) {
        if let Some(id) = self.job.borrow_mut().take() {
            id.remove();
        }
        if let Some(picture) = self
            .stack
            .child_by_name("image")
            .and_downcast::<gtk::Picture>()
        {
            picture.set_paintable(gtk::gdk::Paintable::NONE);
        }
        self.source.replace(source.clone());
        self.stack.set_visible_child_name(if source.is_some() {
            "loading"
        } else {
            "missing"
        });
        if self.stack.is_mapped()
            && let Some(source) = source
        {
            let id = begin_preview(&self.stack, source, self.edge, self.crop, self.job.clone());
            self.job.replace(Some(id));
        }
    }
}

fn begin_preview(
    stack: &gtk::Stack,
    source: Source,
    edge: i32,
    crop: bool,
    job: Rc<RefCell<Option<glib::SourceId>>>,
) -> glib::SourceId {
    let (send, receive) = mpsc::sync_channel(1);
    let alive = Arc::new(());
    let request = Request {
        source,
        edge,
        alive: Arc::downgrade(&alive),
        reply: send,
    };
    let mut pending = Some(request);
    let weak = stack.downgrade();
    glib::timeout_add_local(Duration::from_millis(20), move || {
        #[cfg(test)]
        PREVIEW_POLLS.fetch_add(1, Ordering::Relaxed);
        let _keep_alive = &alive;
        let Some(stack) = weak.upgrade() else {
            job.borrow_mut().take();
            return glib::ControlFlow::Break;
        };
        if let Some(request) = pending.take() {
            match worker().try_send(request) {
                Ok(()) => (),
                Err(mpsc::TrySendError::Full(request)) => {
                    pending = Some(request);
                    return glib::ControlFlow::Continue;
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    stack.set_visible_child_name("missing");
                    job.borrow_mut().take();
                    return glib::ControlFlow::Break;
                }
            }
        }
        match receive.try_recv() {
            Ok(Some(pixels)) => {
                let pixbuf = Pixbuf::from_bytes(
                    &glib::Bytes::from_owned(SharedPixels(pixels.clone())),
                    Colorspace::Rgb,
                    pixels.alpha,
                    8,
                    pixels.width,
                    pixels.height,
                    pixels.stride,
                );
                let pixbuf = if crop {
                    let side = pixbuf.width().min(pixbuf.height());
                    pixbuf.new_subpixbuf(
                        (pixbuf.width() - side) / 2,
                        (pixbuf.height() - side) / 2,
                        side,
                        side,
                    )
                } else {
                    pixbuf
                };
                // Do not retain the picture from this timer: dropping the page cancels work.
                if let Some(picture) = stack.child_by_name("image").and_downcast::<gtk::Picture>() {
                    picture.set_pixbuf(Some(&pixbuf));
                }
                stack.set_visible_child_name("image");
                job.borrow_mut().take();
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            _ => {
                stack.set_visible_child_name("missing");
                job.borrow_mut().take();
                glib::ControlFlow::Break
            }
        }
    })
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
    fn cache_reuses_pixels_but_revalidates_changes_and_symlinks() {
        let (dir, source) = fixture(PNG);
        let mut cache = PreviewCache::default();
        let first = cache.load(&source, 256).unwrap();
        assert!(Arc::ptr_eq(&first, &cache.load(&source, 256).unwrap()));
        let mut changed = PNG.to_vec();
        changed[100] ^= 1;
        std::fs::write(dir.path().join("image.png"), changed).unwrap();
        assert!(cache.load(&source, 256).is_err());
        std::fs::remove_file(dir.path().join("image.png")).unwrap();
        assert!(cache.load(&source, 256).is_err());
        std::fs::write(dir.path().join("other.png"), PNG).unwrap();
        std::os::unix::fs::symlink("other.png", dir.path().join("image.png")).unwrap();
        assert!(cache.load(&source, 256).is_err());
    }

    #[test]
    fn cache_evicts_old_entries_without_invalidating_visible_pixels() {
        let (_dir, source) = fixture(PNG);
        let mut cache = PreviewCache::default();
        let visible = cache.load(&source, 600).unwrap();
        let count = CACHE_BYTES / visible.bytes.len() + 2;
        for edge in 601..=600 + count as i32 {
            cache.load(&source, edge).unwrap();
        }
        assert!(cache.bytes <= CACHE_BYTES);
        assert!(cache.entries.len() < count);
        assert_eq!(visible.width, 512);
        assert!(!Arc::ptr_eq(&visible, &cache.load(&source, 600).unwrap()));
    }

    #[test]
    fn zoom_rejects_non_finite_and_out_of_range_input() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 0.0] {
            assert_eq!(viewer::bounded_zoom(value), 1.0);
        }
        assert_eq!(viewer::bounded_zoom(99.0), 4.0);
        assert_eq!(viewer::bounded_zoom(2.5), 2.5);
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
