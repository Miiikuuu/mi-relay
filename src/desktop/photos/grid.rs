use super::*;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Photo {
    pub id: String,
    pub name: String,
    pub source: Option<Source>,
}

impl Photo {
    fn from_entry(entry: &FileEntry, root: &Path) -> Self {
        let source = if let FileEntry::Saved(record) = entry {
            record
                .stored_path
                .strip_prefix(root)
                .ok()
                .and_then(Path::to_str)
                .map(|relative| Source {
                    root: root.to_owned(),
                    relative: relative.to_owned(),
                    hash: record.sha256.clone(),
                    size: record.size,
                })
        } else {
            None
        };
        Self {
            id: entry.id().to_owned(),
            name: entry.name().to_owned(),
            source,
        }
    }
}

#[derive(Clone)]
pub(in crate::desktop) struct AlbumGrid {
    pub widget: gtk::ScrolledWindow,
    pub view: gtk::GridView,
    store: gio::ListStore,
    photos: Rc<RefCell<Vec<Photo>>>,
}

impl AlbumGrid {
    pub fn new(parent: &adw::ApplicationWindow) -> Self {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::new(Some(store.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        let factory = gtk::SignalListItemFactory::new();
        // Reuse the actual image/placeholder/spinner widgets as ListItems recycle,
        // not just their outer GTK containers. Never retain the ListItem itself.
        let cells = Rc::new(RefCell::new(HashMap::<usize, Preview>::new()));
        let setup = cells.clone();
        factory.connect_setup(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let preview = Preview::new(256, 128, true);
            preview.stack.set_size_request(128, 128);
            let frame = gtk::AspectFrame::new(0.5, 0.5, 1.0, false);
            frame.set_child(Some(&preview.stack));
            item.set_child(Some(&frame));
            item.set_activatable(true);
            setup.borrow_mut().insert(item.as_ptr() as usize, preview);
        });
        let bound = cells.clone();
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let object = item.item().and_downcast::<glib::BoxedAnyObject>().unwrap();
            let photo = object.borrow::<Photo>();
            let frame = item.child().unwrap();
            frame.set_tooltip_text(Some(&photo.name));
            frame.update_property(&[gtk::accessible::Property::Label(&format!(
                "Open photo: {}",
                photo.name
            ))]);
            if let Some(preview) = bound.borrow().get(&(item.as_ptr() as usize)) {
                preview.bind(photo.source.clone());
            }
        });
        let unbound = cells.clone();
        factory.connect_unbind(move |_, item| {
            if let Some(preview) = unbound.borrow().get(&(item.as_ptr() as usize)) {
                preview.bind(None);
            }
        });
        factory.connect_teardown(move |_, item| {
            cells.borrow_mut().remove(&(item.as_ptr() as usize));
            item.downcast_ref::<gtk::ListItem>()
                .unwrap()
                .set_child(gtk::Widget::NONE);
        });
        let view = gtk::GridView::new(Some(selection), Some(factory));
        view.set_min_columns(2);
        view.set_max_columns(8);
        view.set_single_click_activate(true);
        view.add_css_class("album-grid");
        view.set_widget_name("photo-grid");
        let widget = gtk::ScrolledWindow::builder()
            .child(&view)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let overlay_preference = widget.is_overlay_scrolling();
        widget.connect_map(move |widget| {
            // The actual native renderer is available after realization, including
            // automatic Cairo fallback. Do not infer it from GSK_RENDERER. Fading
            // an overlay repeatedly repaints the photo wall in Cairo; a regular
            // scrollbar avoids that work while retaining drag/wheel/keyboard input.
            // Re-evaluate on remap without timers or a strong parent reference.
            let allow_overlay = widget
                .native()
                .and_then(|native| native.renderer())
                .is_some_and(|renderer| !renderer.is::<gtk::gsk::CairoRenderer>());
            widget.set_overlay_scrolling(overlay_preference && allow_overlay);
        });
        let photos = Rc::new(RefCell::new(Vec::<Photo>::new()));
        let pages = photos.clone();
        let parent = parent.downgrade();
        view.connect_activate(move |_, position| {
            if let Some(parent) = parent.upgrade() {
                viewer::open(&parent, pages.borrow().clone(), position as usize);
            }
        });
        Self {
            widget,
            view,
            store,
            photos,
        }
    }

    pub fn clear(&self) {
        if let Some(model) = self.view.model() {
            model.unselect_all();
        }
        self.photos.borrow_mut().clear();
        self.store.remove_all();
        self.widget.vadjustment().set_value(0.0);
    }

    pub fn set_entries(&self, entries: &[FileEntry], root: &Path) {
        let photos: Vec<_> = entries
            .iter()
            .filter(|entry| eligible(entry))
            .map(|entry| Photo::from_entry(entry, root))
            .collect();
        let old = self.photos.borrow();
        if *old == photos {
            return;
        }
        // Preserve common objects on pagination/progress, so completed thumbnails
        // don't reload whenever a neighbouring transfer changes state.
        let prefix = old.iter().zip(&photos).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(photos[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let remove = old.len() - prefix - suffix;
        let added: Vec<_> = photos[prefix..photos.len() - suffix]
            .iter()
            .cloned()
            .map(glib::BoxedAnyObject::new)
            .collect();
        drop(old);
        self.photos.replace(photos);
        self.store.splice(prefix as u32, remove as u32, &added);
    }

    pub fn len(&self) -> u32 {
        self.store.n_items()
    }
    pub fn item(&self, index: u32) -> Option<glib::Object> {
        self.store.item(index)
    }
}
