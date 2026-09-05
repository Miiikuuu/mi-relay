use super::*;
use httpmock::prelude::*;

const ADMIN: &str = "isolated-administrator-token-for-ui-tests";
const ID: &str = "00000000-0000-4000-8000-000000000001";

#[test]
fn administrator_paste_trims_only_outer_whitespace() {
    for pasted in [
        ADMIN.to_owned(),
        format!("  {ADMIN} \r\n"),
        format!("\t\u{2003}{ADMIN}\u{00a0}"),
    ] {
        assert_eq!(administrator_credential(&pasted).unwrap(), ADMIN);
    }
    // Do not require hex or change case/punctuation: existing server credentials
    // may be any supported ASCII token, not just the installer's 64 hex digits.
    assert_eq!(
        administrator_credential("  aB+/-_=09  ").unwrap(),
        "aB+/-_=09"
    );
}

#[test]
fn empty_administrator_field_explains_which_credential_is_needed() {
    for value in ["", " \n\t", "\u{2003}\u{00a0}"] {
        let error = administrator_credential(value).unwrap_err().to_string();
        assert!(error.contains("administrator token here"));
        assert!(error.contains("Session Token"));
    }
}

#[test]
fn malformed_administrator_input_is_not_silently_rewritten_or_echoed() {
    for value in [
        format!("{ADMIN} {ADMIN}"),
        format!("{ADMIN}\n{ADMIN}"),
        format!("{ADMIN}\u{200b}"),
        format!("{ADMIN}\0"),
        format!("MIRELAY_ADMIN_TOKEN={ADMIN}"),
        format!("Bearer {ADMIN}"),
        format!("{ADMIN}秘密"),
        "x".repeat(4097),
    ] {
        let error = administrator_credential(&value).unwrap_err().to_string();
        assert!(!error.contains(ADMIN));
        assert!(!error.contains(&value));
    }
    assert!(administrator_credential(&"x".repeat(4096)).is_ok());
}

fn find_widget<T: glib::object::IsA<gtk::Widget> + glib::types::StaticType>(
    root: &gtk::Widget,
    name: &str,
) -> T {
    let mut pending = vec![root.clone()];
    while let Some(widget) = pending.pop() {
        if widget.widget_name() == name {
            return widget.downcast::<T>().expect("wrong widget type");
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            pending.push(widget);
        }
    }
    panic!("missing test widget {name}");
}

#[test]
#[ignore = "requires a graphical session; real GTK Create clicks against isolated HTTP mocks"]
fn gtk_create_keeps_invalid_credentials_and_shows_inline_results() {
    adw::init().unwrap();
    super::super::install_css();
    let server = MockServer::start();
    let directory = tempfile::tempdir().unwrap();
    let dialog = adw::Window::builder()
        .default_width(620)
        .default_height(560)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.add_css_class("settings-page");
    dialog.add_css_class("mirelay");
    let task = SetupTask::new(&dialog, &root);
    let name = gtk::Entry::builder().text("Isolated GTK pairing").build();
    let local = gtk::Entry::builder()
        .text(directory.path().to_str().unwrap())
        .build();
    let url = gtk::Entry::builder().text(server.url("/")).build();
    let receiver = gtk::PasswordEntry::new();
    let insecure = gtk::Switch::builder().active(true).build();
    let group = panel(
        &dialog, &task, &name, &local, &url, &receiver, &insecure, true,
    );
    root.append(&group);
    let clamp = adw::Clamp::builder().maximum_size(560).child(&root).build();
    dialog.set_content(Some(&clamp));
    dialog.present();
    let admin: gtk::PasswordEntry = find_widget(group.upcast_ref(), "pairing-administrator");
    let create: gtk::Button = find_widget(group.upcast_ref(), "pairing-create");
    let feedback: gtk::Label = find_widget(group.upcast_ref(), "pairing-create-feedback");
    let invitation: gtk::Entry = find_widget(group.upcast_ref(), "pairing-invitation");
    let context = glib::MainContext::default();
    let settle = || {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while task.is_busy() || context.pending() {
            assert!(
                std::time::Instant::now() < deadline,
                "GTK setup did not finish"
            );
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    // Optional screenshots contain synthetic credentials only, never live UI.
    let capture = |name: &str| {
        let Some(directory) = std::env::var_os("MIRELAY_PAIRING_TEST_SCREENSHOTS") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        assert!(directory.is_absolute() && directory.is_dir());
        let path = directory.join(name);
        assert!(!path.exists(), "do not overwrite previous visual evidence");
        let deadline = std::time::Instant::now() + Duration::from_millis(120);
        while std::time::Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(1));
        }
        super::super::capture_widget(dialog.upcast_ref(), &path).unwrap();
    };
    settle();

    let mut reject = server.mock(|when, then| {
        when.method(POST).path("/api/v1/folders");
        then.status(401).body(ADMIN);
    });
    // Filling Session Token is not a substitute for the administrator field.
    receiver.set_text("untouched-receiver-field");
    create.emit_clicked();
    assert!(!task.is_busy());
    assert!(feedback.is_visible() && feedback.has_css_class("error"));
    assert!(feedback.text().contains("administrator token here"));
    assert!(admin.has_css_class("error"));
    assert_eq!(receiver.text(), "untouched-receiver-field");
    reject.assert_calls(0);
    capture("empty-administrator.png");
    for invalid in [
        format!("{ADMIN} internal space"),
        format!("MIRELAY_ADMIN_TOKEN={ADMIN}"),
    ] {
        admin.set_text(&invalid);
        create.emit_clicked();
        assert!(!task.is_busy());
        assert_eq!(admin.text(), invalid);
        assert!(!feedback.text().contains(ADMIN));
        reject.assert_calls(0);
    }

    // Formatting is normalized for transport; a 401 retains the entered value.
    admin.set_text(&format!("  {ADMIN}  "));
    create.emit_clicked();
    assert!(task.is_busy());
    assert_eq!(create.label().as_deref(), Some("Creating…"));
    create.emit_clicked(); // Busy guard also covers programmatic/rapid double clicks.
    dialog.close();
    assert!(
        dialog.is_visible(),
        "closing must not lose a pending creation result"
    );
    settle();
    reject.assert_calls(1);
    assert_eq!(admin.text(), ADMIN);
    assert!(feedback.is_visible() && feedback.has_css_class("error"));
    assert!(feedback.text().contains("Credential"));
    assert!(!feedback.text().contains(ADMIN));
    assert!(root.is_sensitive() && create.is_sensitive());
    assert_eq!(create.label().as_deref(), Some("Create on server"));
    assert_eq!(receiver.text(), "untouched-receiver-field");
    assert!(invitation.text().is_empty());
    capture("rejected-administrator.png");
    reject.delete();

    // Neither a server failure nor a malformed successful response clears input,
    // overwrites the receiver/invitation, or automatically retries a mutation.
    for (status, body) in [(403, ADMIN), (503, ADMIN), (200, "invalid JSON")] {
        let mut failure = server.mock(|when, then| {
            when.method(POST)
                .path("/api/v1/folders")
                .header("Authorization", format!("Bearer {ADMIN}"));
            then.status(status).body(body);
        });
        create.emit_clicked();
        settle();
        failure.assert_calls(1);
        assert_eq!(admin.text(), ADMIN);
        assert!(feedback.is_visible() && feedback.has_css_class("error"));
        assert!(!feedback.text().contains(ADMIN));
        assert!(root.is_sensitive() && create.is_sensitive());
        assert_eq!(receiver.text(), "untouched-receiver-field");
        assert!(invitation.text().is_empty());
        failure.delete();
    }

    // Connection failures preserve the same input as rejected HTTP responses.
    let disconnected = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    url.set_text(&format!("http://{}", disconnected.local_addr().unwrap()));
    let disconnect = std::thread::spawn(move || {
        use std::io::Read;
        let (mut stream, _) = disconnected.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = [0u8; 2048];
        let _ = stream.read(&mut request);
        // Deliberately close without an HTTP response. Never log request bytes.
    });
    create.emit_clicked();
    settle();
    disconnect.join().unwrap();
    assert_eq!(admin.text(), ADMIN);
    assert!(feedback.text().contains("Connection failed."));
    assert!(feedback.has_css_class("error") && create.is_sensitive());
    assert_eq!(receiver.text(), "untouched-receiver-field");
    assert!(invitation.text().is_empty());
    url.set_text(&server.url("/"));

    let token = "a".repeat(64);
    let code = format!("{ID}.{}", "b".repeat(64));
    let success = server.mock(|when, then| {
        when.method(POST)
            .path("/api/v1/folders")
            .header("Authorization", format!("Bearer {ADMIN}"));
        then.status(200).json_body(serde_json::json!({
            "schema_version": 1, "folder_id": ID, "receiver_token": token,
            "pairing_code": code, "expires_at_unix": 2_000_000_000
        }));
    });
    create.emit_clicked();
    settle();
    success.assert_calls(1);
    assert!(admin.text().is_empty() && !admin.is_sensitive());
    assert!(!create.is_sensitive());
    assert_eq!(create.label().as_deref(), Some("Created"));
    assert!(feedback.is_visible() && !feedback.has_css_class("error"));
    assert!(feedback.text().starts_with("Folder created."));
    assert_eq!(receiver.text(), token);
    assert_eq!(invitation.text(), code);
    assert_eq!(url.text(), format!("{}/f/{ID}", server.base_url()));
    capture("created-folder.png");
    create.emit_clicked();
    settle();
    success.assert_calls(1);
    dialog.close();
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
