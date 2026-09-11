use super::*;
use anyhow::ensure;

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut result = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        result.extend(descendants(&widget));
    }
    result
}

fn names(ui: &DesktopUi) -> Vec<String> {
    descendants(ui.widgets.delivery_list.upcast_ref())
        .iter()
        .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
        .filter(|label| label.has_css_class("delivery-name"))
        .map(|label| label.text().to_string())
        .collect()
}

fn rings(widget: &gtk::Widget) -> usize {
    descendants(widget)
        .iter()
        .filter(|widget| widget.is::<gtk::DrawingArea>())
        .count()
}

fn sort_options(menu: &gtk::MenuButton) -> Result<gtk::Box> {
    let popover = menu.popover().context("missing sort popover")?;
    let child = popover.child().context("missing sort controls")?;
    let options = child
        .downcast::<gtk::Box>()
        .map_err(|_| anyhow::anyhow!("sort controls must use a compact box"))?;
    ensure!(options.has_css_class("sort-options"));
    ensure!(options.orientation() == gtk::Orientation::Vertical);
    Ok(options)
}

fn sort_buttons(menu: &gtk::MenuButton) -> Result<Vec<gtk::ToggleButton>> {
    Ok(descendants(sort_options(menu)?.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::ToggleButton>().ok())
        .collect())
}

// Shared with the sidebar smoke test, so both sort menus exercise the same UI contract.
pub(super) fn check_sort_controls(
    menu: &gtk::MenuButton,
    action: &str,
    expected_fields: &[(&str, [&str; 2])],
) -> Result<()> {
    let options = sort_options(menu)?;
    let rows: Vec<_> = descendants(options.upcast_ref())
        .into_iter()
        .filter(|widget| widget.has_css_class("sort-field"))
        .collect();
    ensure!(
        rows.len() == expected_fields.len(),
        "sort menu duplicated criteria instead of pairing directions"
    );
    for (row, (field, targets)) in rows.iter().zip(expected_fields) {
        let row = row
            .downcast_ref::<gtk::Box>()
            .context("sort criterion must use one horizontal row")?;
        ensure!(row.orientation() == gtk::Orientation::Horizontal);
        let contents = descendants(row.upcast_ref());
        let labels: Vec<_> = contents
            .iter()
            .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
            .map(|label| label.text().to_string())
            .collect();
        ensure!(
            labels == [*field],
            "unexpected sort criterion label: {labels:?}"
        );
        let buttons: Vec<_> = contents
            .iter()
            .filter_map(|widget| widget.downcast_ref::<gtk::ToggleButton>())
            .collect();
        ensure!(
            buttons.len() == 2,
            "{field} needs exactly two direction buttons"
        );
        for (button, target) in buttons.iter().zip(targets) {
            ensure!(button.has_css_class("sort-direction"));
            ensure!(button.action_name().as_deref() == Some(action));
            ensure!(
                button
                    .action_target_value()
                    .as_ref()
                    .and_then(|value| value.str())
                    == Some(*target),
                "{field} triangle has the wrong action target"
            );
            ensure!(
                button
                    .tooltip_text()
                    .is_some_and(|text| !text.trim().is_empty()),
                "{field} triangle needs a descriptive tooltip"
            );
        }
    }
    ensure!(sort_buttons(menu)?.len() == expected_fields.len() * 2);
    Ok(())
}

pub(super) fn assert_sort_selected(menu: &gtk::MenuButton, target: &str) -> Result<()> {
    let selected: Vec<_> = sort_buttons(menu)?
        .into_iter()
        .filter(|button| button.is_active())
        .collect();
    ensure!(
        selected.len() == 1,
        "sort controls must have exactly one selected triangle"
    );
    ensure!(
        selected[0]
            .action_target_value()
            .as_ref()
            .and_then(|value| value.str())
            == Some(target),
        "selected triangle does not match sort state: {target}"
    );
    Ok(())
}

pub(super) fn click_sort_direction(menu: &gtk::MenuButton, target: &str) -> Result<()> {
    let button = sort_buttons(menu)?
        .into_iter()
        .find(|button| {
            button
                .action_target_value()
                .as_ref()
                .and_then(|value| value.str())
                == Some(target)
        })
        .with_context(|| format!("missing sort triangle: {target}"))?;
    ensure!(
        button.is_sensitive(),
        "sort triangle is unexpectedly disabled"
    );
    button.emit_clicked();
    assert_sort_selected(menu, target)?;
    // A second click must not leave a stateful sort action with no selected direction.
    button.emit_clicked();
    assert_sort_selected(menu, target)
}

pub(super) fn check_sort_focus(menu: &gtk::MenuButton, target: &str) -> Result<()> {
    menu.popup();
    let result = (|| -> Result<()> {
        let popover = menu.popover().context("missing sort popover")?;
        ensure!(popover.is_mapped(), "sort popover did not open");
        assert_sort_selected(menu, target)?;
        let selected = sort_buttons(menu)?
            .into_iter()
            .find(|button| button.is_active())
            .context("missing selected sort triangle")?;
        // Check the focus destination inside the window, not whether the
        // compositor has granted this disposable preview global input focus.
        ensure!(
            selected.is_focus(),
            "opening sort controls did not focus the selected triangle: {target}"
        );
        Ok(())
    })();
    // Always close the popover, including when a focus assertion fails.
    menu.popdown();
    result
}

// Invoked only by --file-smoke-test. All fixture/progress changes stay in memory.
pub(super) fn run(ui: &Rc<DesktopUi>) -> Result<()> {
    ensure!(!ui.busy.get(), "wait for preview loading");
    let w = &ui.widgets;
    let original_bridges = ui.bridges.borrow().clone();
    let original_selection = ui.selected_bridge_id.borrow().clone();
    let original_views = ui.file_views.borrow().clone();
    let original_activity = ui.file_activity.borrow().clone();
    let original_unread = ui.unread_folders.borrow().clone();
    let original_failed = ui.sync_failed_folders.borrow().clone();
    let original_batching = ui.processing_worker_batch.get();
    let original_error = (w.error_label.text(), w.error_revealer.reveals_child());
    let original_registry = ui.paths.registry.load()?;
    let result = (|| -> Result<()> {
        let selected = ui.selected_bridge().context("select a populated Folder")?;
        let id = selected.registration.id;
        let other = original_bridges
            .iter()
            .find(|view| view.registration.id != id && view.snapshot.is_some())
            .context("provide at least two healthy preview Folders")?
            .registration
            .id
            .clone();
        let mut template = selected
            .snapshot
            .context("selected Folder has no snapshot")?
            .deliveries
            .first()
            .cloned()
            .context("selected Folder needs a sample file")?;
        template.delivery_status = DeliveryStatus::Acknowledged;
        template.wallpaper_status = WallpaperStatus::NotApplicable;
        template.delivery_error = None;
        template.wallpaper_error = None;
        template.media_type = "text/plain".into();
        let mut records: Vec<_> = (0..105)
            .map(|index| {
                let mut record = template.clone();
                record.id = format!("smoke-{index:03}");
                record.original_name = format!("Smoke {index:03}.txt");
                record.size = 100 + index;
                record.received_at_unix = 1000 + index;
                record
            })
            .collect();
        let completed = records[0].clone();
        let mut pending = template.clone();
        pending.id = "smoke-pending".into();
        pending.original_name = "Pending image.png".into();
        pending.media_type = "image/png".into();
        pending.delivery_status = DeliveryStatus::AckPending;
        pending.wallpaper_status = WallpaperStatus::Pending;
        let mut failed = template.clone();
        failed.id = "smoke-failed".into();
        failed.original_name = "Failed archive.zip".into();
        failed.media_type = "application/zip".into();
        failed.delivery_error = Some("in-memory failure".into());
        records.extend([pending.clone(), failed]);
        {
            let mut bridges = ui.bridges.borrow_mut();
            let view = bridges
                .iter_mut()
                .find(|view| view.registration.id == id)
                .unwrap();
            view.snapshot.as_mut().unwrap().deliveries = Arc::new(records.into());
        }
        ui.file_views.borrow_mut().remove(&id);
        ui.file_views.borrow_mut().remove(&other);
        ui.file_activity.borrow_mut().clear();
        ui.render_current_bridge();
        check_sort_controls(
            &w.file_sort_button,
            "win.file-sort",
            &[
                ("Name", ["name-asc", "name-desc"]),
                ("Date received", ["oldest", "newest"]),
                ("Size", ["smallest", "largest"]),
            ],
        )?;
        check_sort_focus(&w.file_sort_button, "newest")?;
        let activate = |name: &str, target: Option<&str>| -> Result<()> {
            let action = w
                .window
                .lookup_action(name)
                .context("missing file action")?;
            action.activate(target.map(|value| value.to_variant()).as_ref());
            if let Some(target) = target {
                ensure!(action.state().as_ref().and_then(|state| state.str()) == Some(target));
            }
            Ok(())
        };
        ensure!(names(ui).len() == FILE_PAGE_SIZE && names(ui).len() > 12);
        let first_row = w.delivery_list.row_at_index(0);
        ui.render_current_bridge();
        ensure!(
            w.delivery_list.row_at_index(0) == first_row,
            "unchanged history rebuilt GTK rows"
        );
        ui.selected_bridge_id.replace(None);
        ui.render_current_bridge();
        ensure!(names(ui).is_empty());
        ui.selected_bridge_id.replace(Some(id.clone()));
        ui.render_current_bridge();
        ensure!(
            names(ui).len() == FILE_PAGE_SIZE,
            "empty-state transition retained a stale row cache"
        );
        ensure!(w.file_show_more.is_visible(), "missing Show More");
        activate("more-files", None)?;
        ensure!(names(ui).len() == 107 && !w.file_show_more.is_visible());
        activate("file-filter", Some("completed"))?;
        for (sort, first) in [
            ("newest", 104),
            ("oldest", 0),
            ("name-asc", 0),
            ("name-desc", 104),
            ("largest", 104),
            ("smallest", 0),
        ] {
            click_sort_direction(&w.file_sort_button, sort)?;
            ensure!(names(ui).first() == Some(&format!("Smoke {first:03}.txt")));
        }
        activate("file-sort", Some("oldest"))?;
        assert_sort_selected(&w.file_sort_button, "oldest")?;
        activate("file-filter", Some("in-progress"))?;
        activate("file-kind", Some("images"))?;
        ensure!(names(ui) == [pending.original_name.clone()]);
        ensure!(
            rings(w.delivery_list.upcast_ref()) == 0,
            "pending must not animate"
        );
        activate("file-kind", Some("archives"))?;
        ensure!(names(ui).is_empty());
        ensure!(w.activity_stack.visible_child_name().as_deref() == Some("no-results"));
        activate("file-filter", Some("attention"))?;
        ensure!(names(ui) == ["Failed archive.zip"]);
        activate("clear-file-filters", None)?;
        w.file_search.set_text("  sMoKe 104  ");
        ensure!(names(ui) == ["Smoke 104.txt"]);
        w.file_search.set_text("no-such-smoke-file");
        ensure!(names(ui).is_empty());
        activate("clear-file-filters", None)?;
        ensure!(names(ui).len() == FILE_PAGE_SIZE && w.file_search.text().is_empty());
        activate("file-sort", Some("name-desc"))?;
        assert_sort_selected(&w.file_sort_button, "name-desc")?;
        activate("file-filter", Some("completed"))?;
        activate("file-kind", Some("documents"))?;
        w.file_search.set_text("Smoke 104");
        ui.selected_bridge_id.replace(Some(other.clone()));
        ui.render_current_bridge();
        assert_sort_selected(&w.file_sort_button, "newest")?;
        ensure!(
            w.file_search.text().is_empty(),
            "query leaked between Folders"
        );
        activate("file-filter", Some("attention"))?;
        click_sort_direction(&w.file_sort_button, "oldest")?;
        ui.selected_bridge_id.replace(Some(id.clone()));
        ui.render_current_bridge();
        check_sort_focus(&w.file_sort_button, "name-desc")?;
        let view = ui.file_views.borrow().get(&id).cloned().unwrap();
        ensure!(
            view.query == "Smoke 104"
                && view.sort == FileSort::NameDesc
                && view.filter == FileFilter::Completed
                && view.kind == FileKind::Documents,
            "Folder view state lost"
        );
        ensure!(names(ui) == ["Smoke 104.txt"]);
        ensure!(ui.file_views.borrow()[&other].filter == FileFilter::Attention);
        ensure!(ui.file_views.borrow()[&other].sort == FileSort::Oldest);
        activate("clear-file-filters", None)?;
        ensure!(ui.begin_task("Receiving…"), "failed to enter busy state");
        let emit = |event| {
            ui.handle_worker_message(WorkerMessage::FileProgress {
                bridge_id: id.clone(),
                event,
            })
        };
        let active = |file_id: &str, name: &str| SyncEvent::FileActive {
            id: file_id.into(),
            original_name: name.into(),
            media_type: "text/plain".into(),
            size: 100,
            phase: SyncPhase::Downloading,
        };
        let before_batch = names(ui);
        ui.processing_worker_batch.set(true);
        emit(active(&completed.id, &completed.original_name));
        ensure!(
            names(ui) == before_batch && ui.file_render_pending.get(),
            "phase events should defer row rebuilding within a batch"
        );
        ui.processing_worker_batch.set(false);
        ui.flush_file_render();
        ensure!(
            !ui.file_render_pending.get(),
            "batch render was not flushed"
        );
        let active_row = w.delivery_list.row_at_index(0);
        emit(active(&completed.id, &completed.original_name));
        ensure!(
            w.delivery_list.row_at_index(0) == active_row && !ui.file_render_pending.get(),
            "identical phase update rebuilt rows"
        );
        let second_row = w.delivery_list.row_at_index(1);
        emit(SyncEvent::FileActive {
            id: completed.id.clone(),
            original_name: completed.original_name.clone(),
            media_type: "text/plain".into(),
            size: 100,
            phase: SyncPhase::Verifying,
        });
        ensure!(
            w.delivery_list.row_at_index(0) != active_row,
            "changed phase did not update its row"
        );
        ensure!(
            w.delivery_list.row_at_index(1) == second_row,
            "phase change rebuilt an unaffected row"
        );
        emit(active(&completed.id, &completed.original_name));
        for sort in [
            "newest",
            "oldest",
            "name-asc",
            "name-desc",
            "largest",
            "smallest",
        ] {
            click_sort_direction(&w.file_sort_button, sort)?;
            ensure!(
                names(ui).first() == Some(&completed.original_name),
                "active not first: {sort}"
            );
            ensure!(rings(w.delivery_list.upcast_ref()) == 1);
            ensure!(ui.busy.get(), "progress prematurely finished Receive");
            ensure!(w.sync_indicator.visible_child_name().as_deref() == Some("spinner"));
        }
        emit(SyncEvent::FileSettled {
            id: completed.id.clone(),
            record: Some(Box::new(completed.clone())),
            error: None,
        });
        ensure!(rings(w.delivery_list.upcast_ref()) == 0);
        ensure!(!ui.file_activity.borrow()[&id].contains_key(&completed.id));
        let saved = ui.selected_bridge().unwrap().snapshot.unwrap().deliveries;
        ensure!(
            saved
                .iter()
                .filter(|record| record.id == completed.id)
                .count()
                == 1
        );
        let completed_row = delivery_row(&FileEntry::Saved(completed));
        let content = completed_row.child().unwrap();
        let text = content.first_child().unwrap().next_sibling().unwrap();
        ensure!(
            text.next_sibling().is_none(),
            "completed row has a status/checkmark column"
        );
        ensure!(
            !descendants(completed_row.upcast_ref())
                .iter()
                .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
                .any(|label| matches!(label.text().as_str(), "Receive" | "Received")),
            "completed row retained a Receive label"
        );
        emit(active("smoke-transient-failure", "Transfer failure.txt"));
        emit(SyncEvent::FileSettled {
            id: "smoke-transient-failure".into(),
            record: None,
            error: Some("injected failure".into()),
        });
        activate("file-filter", Some("attention"))?;
        ensure!(names(ui).contains(&"Transfer failure.txt".into()));
        ensure!(rings(w.delivery_list.upcast_ref()) == 0);
        emit(active("smoke-fatal", "Fatal transfer.txt"));
        ui.handle_worker_message(WorkerMessage::Synced {
            automatic: true,
            outcomes: vec![SyncOutcome {
                bridge_id: id.clone(),
                bridge_name: "Smoke Folder".into(),
                result: Err("injected fatal sync error".into()),
            }],
        });
        ensure!(!ui.busy.get(), "fatal sync did not stop Receive");
        ensure!(w.sync_indicator.visible_child_name().as_deref() == Some("icon"));
        ensure!(
            ui.file_activity.borrow()[&id]
                .values()
                .all(|activity| activity.phase.is_none()),
            "fatal sync left live phases"
        );
        ensure!(names(ui).contains(&"Fatal transfer.txt".into()));
        ensure!(rings(w.delivery_list.upcast_ref()) == 0);
        ensure!(ui.paths.registry.load()? == original_registry);
        // Directory mode uses the same filtering and quiet completed rows,
        // with explicit retained-copy metadata arriving in the final snapshot.
        let applied = crate::directory::receiver::Applied {
            version: 7,
            sha256: "a".repeat(64),
            conflict: true,
            history: Some("Folder/.mirelay-history-00000000-0000-4000-8000-000000000001".into()),
            acknowledged: true,
            size: 42,
            media_type: "text/plain".into(),
            received_at_unix: 1_700_000_000,
        };
        let quiet = crate::directory::receiver::Applied {
            version: 8,
            conflict: false,
            history: None,
            ..applied.clone()
        };
        let mut directory_snapshot = ui
            .bridges
            .borrow()
            .iter()
            .find(|bridge| bridge.registration.id == id)
            .unwrap()
            .snapshot
            .clone()
            .unwrap();
        let files = std::collections::BTreeMap::from([
            ("Folder/conflict.txt".to_owned(), applied),
            ("Folder/complete.txt".to_owned(), quiet),
        ]);
        directory_snapshot.deliveries = Arc::new(
            files
                .iter()
                .map(|(path, file)| {
                    directory_panel::record(&directory_snapshot.library_dir, path, file)
                })
                .collect::<Vec<_>>()
                .into(),
        );
        directory_snapshot.directory = Some(Arc::new(directory_panel::DirectorySnapshot { files }));
        ui.file_activity.borrow_mut().remove(&id);
        ui.file_views
            .borrow_mut()
            .insert(id.clone(), FileViewOptions::default());
        ui.selected_bridge_id.replace(Some(id.clone()));
        ui.update_bridge_snapshot(&id, directory_snapshot);
        ui.render_current_bridge();
        ensure!(names(ui).len() == 2);
        let buttons: Vec<_> = descendants(w.delivery_list.upcast_ref())
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
            .filter(|button| {
                button.tooltip_text().as_deref() == Some("Retained copy for this version")
            })
            .collect();
        ensure!(
            buttons.len() == 1,
            "final snapshot must render retained-copy action even after settlement"
        );
        buttons[0].emit_clicked();
        let history = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::MessageDialog>().ok())
            .find(|dialog| dialog.text().as_deref() == Some("Conflict copy kept"))
            .context("retained copy dialog did not open")?;
        ensure!(
            history
                .secondary_text()
                .is_some_and(|text| text.contains("Folder/.mirelay-history-")
                    && text.contains("Source version 7"))
        );
        history.close();
        activate("file-filter", Some("attention"))?;
        ensure!(names(ui) == ["Folder/conflict.txt"]);
        activate("file-filter", Some("completed"))?;
        ensure!(names(ui) == ["Folder/complete.txt"]);
        ensure!(rings(w.delivery_list.upcast_ref()) == 0);
        let editor = show_bridge_editor(ui, BridgeEditorMode::New)
            .context("new Folder editor did not open")?;
        let mode = descendants(editor.upcast_ref())
            .into_iter()
            .find(|widget| widget.widget_name() == "directory-mode")
            .unwrap()
            .downcast::<gtk::Switch>()
            .unwrap();
        ensure!(
            !mode.is_active(),
            "existing delivery behavior stays the default"
        );
        mode.set_active(true);
        ensure!(
            descendants(editor.upcast_ref())
                .iter()
                .filter_map(|widget| widget.downcast_ref::<gtk::Button>())
                .any(|button| button.label().as_deref() == Some("Review directory…"))
        );
        // Real Add Folder layout: a missing admin credential must be reported
        // beside Create, before any request or registration can be attempted.
        let editor_widgets = descendants(editor.upcast_ref());
        let create = editor_widgets
            .iter()
            .find(|widget| widget.widget_name() == "pairing-create")
            .and_then(|widget| widget.downcast_ref::<gtk::Button>())
            .context("missing pairing create button")?;
        create.emit_clicked();
        let feedback = editor_widgets
            .iter()
            .find(|widget| widget.widget_name() == "pairing-create-feedback")
            .and_then(|widget| widget.downcast_ref::<gtk::Label>())
            .context("missing inline pairing feedback")?;
        ensure!(feedback.is_visible() && feedback.has_css_class("error"));
        ensure!(feedback.text().contains("administrator token here"));
        ensure!(create.is_sensitive());
        editor.close();
        ensure!(!ui.editor_open.get());
        ensure!(
            ui.paths.registry.load()? == original_registry,
            "opening/cancelling mode selection must not initialize anything"
        );
        // Local category changes must work without a network or credentials.
        let active_id = ui
            .selected_bridge_id
            .borrow()
            .clone()
            .context("selected Folder")?;
        let mut image = template.clone();
        image.id = "category-photo".into();
        image.original_name = "Photo.PNG".into();
        image.media_type = "image/png".into();
        let mut pending_image = image.clone();
        pending_image.id = "category-pending".into();
        pending_image.original_name = "Waiting.png".into();
        pending_image.delivery_status = DeliveryStatus::AckPending;
        {
            let mut bridges = ui.bridges.borrow_mut();
            let bridge = bridges
                .iter_mut()
                .find(|bridge| bridge.registration.id == active_id)
                .unwrap();
            bridge.snapshot.as_mut().unwrap().deliveries =
                Arc::new(vec![image, pending_image, template.clone()].into());
        }
        ui.file_views.borrow_mut().remove(&active_id);
        ui.file_activity.borrow_mut().remove(&active_id);
        w.folder_category.set_selected(1);
        ui.render_current_bridge();
        ensure!(
            ui.paths
                .registry
                .load()?
                .bridges
                .iter()
                .find(|bridge| bridge.id == active_id)
                .unwrap()
                .kind
                == crate::bridge_registry::FolderKind::Photos
        );
        ensure!(
            w.photo_grid.child_at_index(0).is_some() && w.photo_grid.child_at_index(1).is_none()
        );
        ensure!(names(ui).contains(&"Waiting.png".to_owned()));
        ensure!(!names(ui).contains(&"Photo.PNG".to_owned()));
        let tile = w.photo_grid.child_at_index(0).unwrap();
        ui.render_current_bridge();
        ensure!(
            w.photo_grid.child_at_index(0).as_ref() == Some(&tile),
            "unchanged gallery should retain thumbnails"
        );
        ui.handle_file_progress(
            &active_id,
            SyncEvent::FileActive {
                id: "category-active".into(),
                original_name: "Downloading.png".into(),
                media_type: "image/png".into(),
                size: 1024,
                phase: SyncPhase::Downloading,
            },
        );
        ensure!(names(ui).first().map(String::as_str) == Some("Downloading.png"));
        ensure!(
            w.photo_grid.child_at_index(0).as_ref() == Some(&tile),
            "transfer progress rebuilt completed thumbnails"
        );
        ui.file_activity.borrow_mut().remove(&active_id);
        w.folder_category.set_selected(0);
        ensure!(w.photo_grid.child_at_index(0).is_none());
        ensure!(names(ui).contains(&"Photo.PNG".to_owned()));
        ensure!(
            ui.paths.registry.load()? == original_registry,
            "category roundtrip changed other settings"
        );
        Ok(())
    })();
    ui.bridges.replace(original_bridges);
    ui.selected_bridge_id.replace(original_selection);
    ui.file_views.replace(original_views);
    ui.file_activity.replace(original_activity);
    ui.unread_folders.replace(original_unread);
    ui.sync_failed_folders.replace(original_failed);
    ui.processing_worker_batch.set(original_batching);
    ui.file_render_pending.set(false);
    ui.finish_task();
    ui.render_bridge_list();
    w.error_label.set_label(&original_error.0);
    w.error_revealer.set_reveal_child(original_error.1);
    result?;
    if std::env::var_os("MIRELAY_DESKTOP_STRESS").as_deref() == Some(std::ffi::OsStr::new("1")) {
        super::stress_smoke::run(ui)?;
    }
    Ok(())
}
