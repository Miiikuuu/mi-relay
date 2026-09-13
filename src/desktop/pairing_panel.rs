use super::*;
use crate::pairing::PairingClient;
mod qr;

// Normalize pasted input at the UI boundary, not inside the shared protocol
// client. Never remove characters from the middle of a credential or echo it.
fn administrator_credential(value: &str) -> Result<&str> {
    let value = value.trim();
    anyhow::ensure!(
        !value.is_empty(),
        "Paste the administrator token here. Session Token is filled after creation."
    );
    anyhow::ensure!(value.len() <= 4096, "Administrator credential is too long.");
    anyhow::ensure!(
        !value.starts_with("MIRELAY_ADMIN_TOKEN=") && !value.starts_with("Bearer "),
        "Paste only the token value, without MIRELAY_ADMIN_TOKEN= or Bearer."
    );
    anyhow::ensure!(
        value.bytes().all(|byte| byte.is_ascii_graphic()),
        "The token contains an internal space, line break or non-ASCII character. Paste only the token value."
    );
    Ok(value)
}

fn create_feedback(label: &gtk::Label, message: &str, error: bool) {
    label.set_label(&safe_ui_message(message, 1000));
    label.set_visible(true);
    if error {
        label.add_css_class("error");
    } else {
        label.remove_css_class("error");
    }
}

/// Setup traffic never runs on GTK's main thread. Closing while a credential is
/// being issued is blocked so the response cannot disappear with its dialog.
#[derive(Clone)]
pub(super) struct SetupTask {
    root: gtk::Box,
    busy: Rc<Cell<bool>>,
}

impl SetupTask {
    pub fn is_busy(&self) -> bool {
        self.busy.get()
    }
    pub fn new(dialog: &adw::Window, root: &gtk::Box) -> Self {
        let busy = Rc::new(Cell::new(false));
        let pending = busy.clone();
        dialog.connect_close_request(move |_| {
            if pending.get() {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        Self {
            root: root.clone(),
            busy,
        }
    }

    pub fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T> + Send + 'static,
        done: impl FnOnce(Result<T>) + 'static,
    ) {
        if self.busy.replace(true) {
            return;
        }
        self.root.set_sensitive(false);
        let this = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(work).await.unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "Setup stopped safely. Check the connection and try again."
                ))
            });
            this.busy.set(false);
            this.root.set_sensitive(true);
            done(result);
        });
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn panel(
    dialog: &adw::Window,
    task: &SetupTask,
    name: &gtk::Entry,
    local: &gtk::Entry,
    url: &gtk::Entry,
    token: &gtk::PasswordEntry,
    insecure: &gtk::Switch,
    is_new: bool,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Pairing")
        .description("Create an invitation on Linux, join on Android, then compare the verification code. Files stay blocked until confirmed.").build();
    let admin = gtk::PasswordEntry::builder()
        .placeholder_text("Paste administrator token")
        .show_peek_icon(true)
        .width_chars(14)
        .max_width_chars(22)
        .valign(gtk::Align::Center)
        .build();
    admin.set_widget_name("pairing-administrator");
    let create = gtk::Button::with_label("Create on server");
    create.set_widget_name("pairing-create");
    create.set_valign(gtk::Align::Center);
    let feedback = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(32)
        .visible(false)
        .build();
    feedback.set_widget_name("pairing-create-feedback");
    feedback.add_css_class("caption");
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.append(&admin);
    controls.append(&create);
    let input = gtk::Box::new(gtk::Orientation::Vertical, 6);
    input.set_valign(gtk::Align::Center);
    input.set_margin_top(8);
    input.set_margin_bottom(8);
    input.append(&controls);
    input.append(&feedback);
    let create_row = adw::ActionRow::builder()
        .title("Administrator credential")
        .subtitle("Creates a new Folder on the server. Never share with Android.")
        .subtitle_lines(2)
        .build();
    create_row.add_suffix(&input);
    create_row.set_visible(is_new);
    group.add(&create_row);
    let code = gtk::Entry::builder()
        .editable(false)
        .placeholder_text("Pairing code appears here")
        .hexpand(true)
        .build();
    code.set_widget_name("pairing-invitation");
    code.set_tooltip_text(Some(
        "Private, single-device invitation. Copy to Android; expires after 10 minutes.",
    ));
    let invitation_row = adw::ActionRow::builder().title("Invitation").build();
    invitation_row.add_suffix(&code);
    let qr = qr::InvitationQr::new();
    invitation_row.add_suffix(&qr.button);
    {
        let qr = qr.clone();
        let code = code.clone();
        url.connect_changed(move |_| {
            qr.clear();
            code.set_text("");
        });
    }
    group.add(&invitation_row);
    let status = gtk::Label::builder()
        .label("Not checked")
        .wrap(true)
        .max_width_chars(36)
        .xalign(0.0)
        .selectable(true)
        .build();
    let status_row = adw::ActionRow::builder().title("Verification").build();
    status_row.add_suffix(&status);
    group.add(&status_row);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let check = gtk::Button::with_label("Check pairing");
    let confirm = gtk::Button::with_label("Codes match — confirm");
    confirm.set_sensitive(false);
    let renew = gtk::Button::with_label("Replace invitation…");
    actions.append(&check);
    actions.append(&confirm);
    actions.append(&renew);
    group.add(&actions);
    let verification = Rc::new(RefCell::new(None::<String>));
    {
        let qr = qr.clone();
        let (task, name, local, url, token, insecure, admin, code, status, create, feedback) = (
            task.clone(),
            name.clone(),
            local.clone(),
            url.clone(),
            token.clone(),
            insecure.clone(),
            admin.clone(),
            code.clone(),
            status.clone(),
            create.clone(),
            feedback.clone(),
        );
        create.clone().connect_clicked(move |_| {
            if task.is_busy() || !create.is_sensitive() {
                return;
            }
            let raw = admin.text();
            let secret = match administrator_credential(raw.as_str()) {
                Ok(secret) => secret.to_owned(),
                Err(error) => {
                    create_feedback(&feedback, &error.to_string(), true);
                    admin.add_css_class("error");
                    admin.grab_focus();
                    return;
                }
            };
            admin.remove_css_class("error");
            let directory = local.text().trim().to_owned();
            if directory.is_empty() || !Path::new(&directory).is_absolute() {
                create_feedback(&feedback, "Choose an absolute local Folder path first.", true);
                local.grab_focus();
                return;
            }
            let base = url.text().trim().trim_end_matches('/').to_owned();
            let display = if name.text().trim().is_empty() { bridge_name_for_path(Path::new(&directory)) } else { name.text().trim().to_owned() };
            admin.set_text(&secret);
            let insecure = insecure.is_active();
            let (url,token,admin,code,status,create,feedback)=(url.clone(),token.clone(),admin.clone(),code.clone(),status.clone(),create.clone(),feedback.clone());
            let qr = qr.clone();
            status.set_label("Creating…");
            create.set_label("Creating…");
            create_feedback(&feedback, "Creating a new Folder…", false);
            task.run(move || { let folder=PairingClient::new(&base,&secret,insecure)?.create_folder(&display)?; Ok((base,folder)) }, move |result| {
                match result {
                    Ok((base,folder)) => {
                        admin.set_text("");
                        admin.set_sensitive(false);
                        url.set_text(&format!("{base}/f/{}",folder.folder_id)); token.set_text(&folder.receiver_token);
                        code.set_text(&folder.pairing_code); create.set_sensitive(false);
                        qr.set(&base, &folder.pairing_code, folder.expires_at_unix);
                        create.set_label("Created");
                        create_feedback(&feedback, "Folder created. Use the invitation below on Android.", false);
                        status.set_label("Awaiting Android. Scan the QR code, or copy the invitation and original server URL. Keep a private copy of the receiver token; it is session-only.");
                    }
                    Err(error) => {
                        create.set_label("Create on server");
                        create_feedback(&feedback, &error.to_string(), true);
                        status.set_label(&safe_ui_message(&error.to_string(),1000));
                    }
                }
            });
        });
    }
    {
        let feedback = feedback.clone();
        let task = task.clone();
        admin.connect_changed(move |admin| {
            admin.remove_css_class("error");
            if !task.is_busy() {
                feedback.set_visible(false);
            }
        });
    }
    {
        let (task, url, token, insecure, status, confirm, verification) = (
            task.clone(),
            url.clone(),
            token.clone(),
            insecure.clone(),
            status.clone(),
            confirm.clone(),
            verification.clone(),
        );
        let qr = qr.clone();
        check.connect_clicked(move |_| {
            let (base, secret, insecure) = (
                url.text().to_string(),
                token.text().to_string(),
                insecure.is_active(),
            );
            let (status, confirm, verification) =
                (status.clone(), confirm.clone(), verification.clone());
            confirm.set_sensitive(false);
            *verification.borrow_mut() = None;
            status.set_label("Checking…");
            let qr = qr.clone();
            task.run(
                move || PairingClient::new(&base, &secret, insecure)?.handshake(),
                move |result| match result {
                    Ok(info) if info.role == "receiver" => {
                        if info.state != "awaiting_peer" {
                            qr.clear();
                        }
                        status.set_label(&format!(
                            "{}{}",
                            info.state.replace('_', " "),
                            info.verification
                                .as_ref()
                                .map(|v| format!(" · {v}\nCompare with Android before confirming."))
                                .unwrap_or_default()
                        ));
                        if info.state == "awaiting_confirmation" {
                            *verification.borrow_mut() = info.verification;
                            confirm.set_sensitive(true);
                        }
                    }
                    Ok(_) => status.set_label("Use a receiver credential on Linux."),
                    Err(error) => status.set_label(&safe_ui_message(&error.to_string(), 1000)),
                },
            );
        });
    }
    {
        let (task, url, token, insecure, status, verification, confirm) = (
            task.clone(),
            url.clone(),
            token.clone(),
            insecure.clone(),
            status.clone(),
            verification.clone(),
            confirm.clone(),
        );
        let qr = qr.clone();
        confirm.clone().connect_clicked(move |_| {
            let Some(value)=verification.borrow().clone() else { return; };
            let (base,secret,insecure)=(url.text().to_string(),token.text().to_string(),insecure.is_active());
            let (status,confirm)=(status.clone(),confirm.clone()); confirm.set_sensitive(false);
            let qr = qr.clone();
            task.run(move || PairingClient::new(&base,&secret,insecure)?.confirm(&value), move |result| match result {
                Ok(_) => { qr.clear(); status.set_label("Ready · receiver confirmed. Enable Auto on Android when you want automatic sending."); },
                Err(error) => status.set_label(&safe_ui_message(&error.to_string(),1000)),
            });
        });
    }
    {
        let (task, dialog, url, token, insecure, status, code, confirm, verification) = (
            task.clone(),
            dialog.clone(),
            url.clone(),
            token.clone(),
            insecure.clone(),
            status.clone(),
            code.clone(),
            confirm.clone(),
            verification.clone(),
        );
        let qr = qr.clone();
        renew.connect_clicked(move |_| {
            let warning = gtk::MessageDialog::builder().transient_for(&dialog).modal(true).message_type(gtk::MessageType::Warning)
                .buttons(gtk::ButtonsType::Cancel).text("Replace invitation?")
                .secondary_text("This disconnects the current sender and blocks new transfers until you confirm a new pairing. Files already received stay unchanged.").build();
            warning.add_button("Disconnect and replace",gtk::ResponseType::Accept);
            let (task,url,token,insecure,status,code,confirm,verification)=(task.clone(),url.clone(),token.clone(),insecure.clone(),status.clone(),code.clone(),confirm.clone(),verification.clone());
            let qr = qr.clone();
            warning.connect_response(move |warning,response| {
                if response == gtk::ResponseType::Accept {
                    let (base,secret,insecure)=(url.text().to_string(),token.text().to_string(),insecure.is_active());
                    let (status,code,confirm,verification)=(status.clone(),code.clone(),confirm.clone(),verification.clone());
                    let qr = qr.clone();
                    qr.clear();
                    let qr_base = base.rsplit_once("/f/").map(|(base,_)| base.to_owned());
                    task.run(move || PairingClient::new(&base,&secret,insecure)?.renew(),move |result| match result {
                        Ok(invite)=>{ code.set_text(&invite.pairing_code); if let Some(base) = &qr_base { qr.set(base, &invite.pairing_code, invite.expires_at_unix); } status.set_label("Awaiting Android · old sender disconnected"); confirm.set_sensitive(false); *verification.borrow_mut()=None; }
                        Err(error)=>status.set_label(&safe_ui_message(&error.to_string(),1000)),
                    });
                }
                warning.close();
            });
            warning.present();
        });
    }
    group
}

#[cfg(test)]
mod tests;
