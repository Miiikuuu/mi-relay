//! A bounded, local-only QR encoder. Never includes either long-lived token.
use super::*;
use qrcode::{Color, EcLevel, QrCode};
use serde::Serialize;

#[derive(Serialize)]
struct Invitation<'a> {
    kind: &'static str,
    version: u8,
    server_url: &'a str,
    pairing_code: &'a str,
    expires_at_unix: u64,
}

pub(super) fn payload(base: &str, code: &str, expires: u64, now: u64) -> Result<String> {
    anyhow::ensure!(
        base.len() <= 512 && base.bytes().all(|b| b.is_ascii_graphic()),
        "Use a shorter HTTPS server URL for QR pairing."
    );
    let url = reqwest::Url::parse(base).map_err(|_| anyhow::anyhow!("Invalid QR server URL."))?;
    let authority = base
        .strip_prefix("https://")
        .unwrap_or_default()
        .split('/')
        .next()
        .unwrap_or_default();
    anyhow::ensure!(
        url.scheme() == "https"
            && url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.port() != Some(0)
            && !authority.contains('@')
            && !authority.ends_with(':')
            && !url.path().contains("/f/")
            && !url.as_str().contains('%')
            && !base.contains(['%', '\\']),
        "QR pairing requires an HTTPS server URL without credentials or a Folder ID."
    );
    let (id, secret) = code.split_once('.').unwrap_or_default();
    anyhow::ensure!(
        uuid::Uuid::parse_str(id).is_ok_and(|v| v.to_string() == id)
            && secret.len() == 64
            && secret.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid invitation."
    );
    anyhow::ensure!(
        expires > now && expires.saturating_sub(now) <= 900,
        "Invitation expired or device clock is incorrect. Replace the invitation."
    );
    Ok(serde_json::to_string(&Invitation {
        kind: "mirelay-pairing",
        version: 1,
        server_url: url.as_str().trim_end_matches('/'),
        pairing_code: code,
        expires_at_unix: expires,
    })?)
}

#[derive(Clone)]
pub(super) struct InvitationQr {
    pub button: gtk::MenuButton,
    area: gtk::DrawingArea,
    note: gtk::Label,
    matrix: Rc<RefCell<Option<QrCode>>>,
    expiry: Rc<Cell<u64>>,
}

impl InvitationQr {
    pub fn new() -> Self {
        let button = gtk::MenuButton::builder()
            .label("Show QR code")
            .sensitive(false)
            .valign(gtk::Align::Center)
            .build();
        button.set_widget_name("pairing-qr");
        let popover = gtk::Popover::new();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let area = gtk::DrawingArea::builder()
            .content_width(300)
            .content_height(300)
            .build();
        area.set_widget_name("pairing-qr-image");
        let matrix: Rc<RefCell<Option<QrCode>>> = Rc::new(RefCell::new(None));
        let data = matrix.clone();
        area.set_draw_func(move |_, ctx, width, height| {
            ctx.set_source_rgb(1.0, 1.0, 1.0);
            let _ = ctx.paint();
            let matrix = data.borrow();
            let Some(qr) = matrix.as_ref() else {
                return;
            };
            let modules = qr.width();
            let scale = (width.min(height) as f64 / (modules + 8) as f64).floor();
            if scale < 1.0 {
                return;
            }
            let left = ((width as f64 - (modules + 8) as f64 * scale) / 2.0).floor() + 4.0 * scale;
            let top = ((height as f64 - (modules + 8) as f64 * scale) / 2.0).floor() + 4.0 * scale;
            ctx.set_antialias(gtk::cairo::Antialias::None);
            ctx.set_source_rgb(0.0, 0.0, 0.0);
            for y in 0..modules {
                for x in 0..modules {
                    if qr[(x, y)] == Color::Dark {
                        ctx.rectangle(
                            left + x as f64 * scale,
                            top + y as f64 * scale,
                            scale,
                            scale,
                        );
                    }
                }
            }
            let _ = ctx.fill();
        });
        let note = gtk::Label::builder().label("In Android: Add Folder → Scan QR code.\nPrivate invitation · expires in 10 minutes.\nCompare codes on both devices before confirming.")
            .wrap(true).max_width_chars(38).build();
        content.append(&area);
        content.append(&note);
        popover.set_child(Some(&content));
        button.set_popover(Some(&popover));
        Self {
            button,
            area,
            note,
            matrix,
            expiry: Rc::new(Cell::new(0)),
        }
    }

    pub fn clear(&self) {
        self.expiry.set(0);
        self.matrix.borrow_mut().take();
        self.button.popdown();
        self.button.set_sensitive(false);
        self.area.queue_draw();
    }

    pub fn set(&self, base: &str, code: &str, expires: u64) {
        self.clear();
        let result = payload(base, code, expires, crate::fsutil::unix_now()).and_then(|text| {
            QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).map_err(Into::into)
        });
        match result {
            Ok(matrix) => {
                *self.matrix.borrow_mut() = Some(matrix);
                self.expiry.set(expires);
                self.button
                    .set_tooltip_text(Some("Private invitation. Do not share screenshots."));
                self.button.set_sensitive(true);
                self.note.set_label("In Android: Add Folder → Scan QR code.\nPrivate invitation · valid until it expires.\nCompare codes on both devices before confirming.");
                self.area.queue_draw();
                let (button, area, data, expiry) = (
                    self.button.downgrade(),
                    self.area.downgrade(),
                    Rc::downgrade(&self.matrix),
                    Rc::downgrade(&self.expiry),
                );
                glib::timeout_add_local_once(
                    Duration::from_secs(expires.saturating_sub(crate::fsutil::unix_now())),
                    move || {
                        if let (Some(button), Some(area), Some(data), Some(expiry)) = (
                            button.upgrade(),
                            area.upgrade(),
                            data.upgrade(),
                            expiry.upgrade(),
                        ) && expiry.get() == expires
                        {
                            data.borrow_mut().take();
                            expiry.set(0);
                            button.popdown();
                            button.set_sensitive(false);
                            button.set_tooltip_text(Some(
                                "Invitation expired. Replace the invitation to scan again.",
                            ));
                            area.queue_draw();
                        }
                    },
                );
            }
            Err(_) => self.button.set_tooltip_text(Some(
                "QR unavailable: use HTTPS and a fresh invitation. Manual entry remains available.",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn code() -> String {
        format!("00000000-0000-4000-8000-000000000001.{}", "ab".repeat(32))
    }
    #[test]
    #[ignore = "manual optical acceptance; explicit private invitation file and agreed monitor required"]
    fn show_private_qr_for_phone_acceptance() {
        let path = std::env::var("MIRELAY_QR_INVITATION_FILE").expect("explicit private input");
        let connector = std::env::var("MIRELAY_QR_MONITOR").expect("explicit agreed display");
        let mut input = std::fs::File::open(path).unwrap();
        use std::io::Read;
        let mut text = String::new();
        input.by_ref().take(4097).read_to_string(&mut text).unwrap();
        assert!(text.len() <= 4096);
        let invitation: serde_json::Value = serde_json::from_str(&text).unwrap();
        let base = invitation["server_url"].as_str().unwrap();
        let code = invitation["pairing_code"].as_str().unwrap();
        let expires = invitation["expires_at_unix"].as_u64().unwrap();
        assert!(
            payload(base, code, expires, crate::fsutil::unix_now()).is_ok(),
            "Fresh HTTPS invitation required"
        );
        adw::init().unwrap();
        let window = adw::Window::builder()
            .title("MiRelay · QR acceptance")
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
        content.set_valign(gtk::Align::Center);
        content.set_halign(gtk::Align::Center);
        content.append(&gtk::Label::new(Some("MiRelay · Private test invitation")));
        content.append(&gtk::Label::new(Some(
            "Android → Add Folder → Scan QR code",
        )));
        let qr = InvitationQr::new();
        qr.set(base, code, expires);
        content.append(&qr.button);
        window.set_content(Some(&content));
        let monitors = gtk::gdk::Display::default().unwrap().monitors();
        let monitor = (0..monitors.n_items())
            .filter_map(|i| monitors.item(i).and_downcast::<gtk::gdk::Monitor>())
            .find(|m| m.connector().as_deref() == Some(connector.as_str()))
            .expect("Requested monitor unavailable");
        window.fullscreen_on_monitor(&monitor);
        let main = glib::MainLoop::new(None, false);
        let quit = main.clone();
        window.connect_close_request(move |_| {
            quit.quit();
            glib::Propagation::Proceed
        });
        window.present();
        let button = qr.button.clone();
        glib::timeout_add_local_once(Duration::from_millis(500), move || button.popup());
        let quit = main.clone();
        glib::timeout_add_local_once(
            Duration::from_secs(expires.saturating_sub(crate::fsutil::unix_now()).min(600)),
            move || quit.quit(),
        );
        println!("Private QR test window shown; input withheld. Close the window to finish.");
        main.run();
        window.close();
    }
    #[test]
    #[ignore = "writes a synthetic QR fixture to the explicitly supplied MIRELAY_QR_FIXTURE path"]
    fn export_linux_qr_fixture() {
        use std::io::Write;
        let path = std::env::var("MIRELAY_QR_FIXTURE").expect("explicit fixture path");
        let text = payload("https://example.com", &code(), 2_000_000_000, 1_999_999_900).unwrap();
        let matrix = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).unwrap();
        let size = (matrix.width() + 8) * 4;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        writeln!(output, "P2\n{size} {size}\n255").unwrap();
        for y in 0..size {
            for x in 0..size {
                let dark = x / 4 >= 4
                    && y / 4 >= 4
                    && x / 4 < matrix.width() + 4
                    && y / 4 < matrix.width() + 4
                    && matrix[(x / 4 - 4, y / 4 - 4)] == Color::Dark;
                write!(output, "{} ", if dark { 0 } else { 255 }).unwrap();
            }
            writeln!(output).unwrap();
        }
    }

    #[test]
    #[ignore = "requires a graphical session; local GTK QR lifetime/expiry test"]
    fn gtk_qr_expiry_invalid_input_and_replacement() {
        adw::init().unwrap();
        let qr = InvitationQr::new();
        assert!(!qr.button.is_sensitive());
        let now = crate::fsutil::unix_now();
        qr.set("https://example.com", &code(), now + 1);
        assert!(qr.button.is_sensitive());
        assert!(qr.matrix.borrow().is_some());
        // An old invitation's timer must not hide a replacement invitation.
        qr.set("https://example.com", &code(), now + 3);
        let main = glib::MainLoop::new(None, false);
        let quit = main.clone();
        let check = qr.clone();
        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            assert!(check.button.is_sensitive());
            quit.quit();
        });
        main.run();
        let quit = main.clone();
        glib::timeout_add_local_once(Duration::from_secs(2), move || quit.quit());
        main.run();
        assert!(!qr.button.is_sensitive());
        assert!(qr.matrix.borrow().is_none());
        qr.set(
            "https://example.com",
            &code(),
            crate::fsutil::unix_now() + 600,
        );
        assert!(qr.button.is_sensitive());
        qr.set(
            "http://example.com",
            &code(),
            crate::fsutil::unix_now() + 600,
        );
        assert!(!qr.button.is_sensitive());
        assert!(qr.matrix.borrow().is_none());
    }
    #[test]
    fn payload_has_only_short_lived_invitation_fields() {
        let text = payload("https://example.com/", &code(), 1600, 1000).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 5);
        assert_eq!(value["server_url"], "https://example.com");
        assert_eq!(value["kind"], "mirelay-pairing");
        assert_eq!(value["pairing_code"], code());
        assert!(QrCode::with_error_correction_level(text, EcLevel::M).is_ok());
    }
    #[test]
    fn rejects_unsafe_urls_expiry_and_invalid_codes_without_echoing() {
        for url in [
            "http://example.com",
            "https://secret@example.com",
            "https://example.com/?token=secret",
            "https://example.com/#secret",
            "https://example.com/f/id",
            "https://example.com/%66/id",
            "https://example.com\\evil",
            "https://example.com/\n",
        ] {
            assert!(payload(url, &code(), 1600, 1000).is_err(), "{url}");
        }
        for expiry in [0, 999, 1000, 1901, u64::MAX] {
            assert!(payload("https://example.com", &code(), expiry, 1000).is_err());
        }
        for bad in ["secret", "", "not-a-uuid.secret"] {
            assert!(payload("https://example.com", bad, 1600, 1000).is_err());
        }
    }
}
