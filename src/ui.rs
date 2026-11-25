//! The picker window: a layer-shell overlay that stays alive and is shown and
//! hidden on request.

use crate::diff;
use crate::history::{self, Entry, Kind};
use crate::keys::{self, Action, Mode};
use crate::marks::Marks;
use crate::thumbs;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::{gdk, gio, glib, pango, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

const WIDTH: i32 = 1100;
const HEIGHT: i32 = 620;
const PREVIEW_WIDTH: i32 = 430;
/// `.frame` border and `paned > separator` width in style.css: the Paned gets
/// WIDTH minus both borders, and the preview must not overflow under the frame.
const FRAME_BORDER: i32 = 2;
const SEPARATOR: i32 = 1;
/// Thumbnails fill a fixed 48×36 box (cropped to it), so labels line up
/// whatever the image's aspect ratio. Loaded large enough to cover it at 2×.
const THUMB_W: i32 = 48;
const THUMB_H: i32 = 36;
const THUMB_LOAD: i32 = 144;
const PREVIEW_LOAD_W: i32 = PREVIEW_WIDTH * 2;
const PREVIEW_LOAD_H: i32 = HEIGHT * 2;
const PAGE: i64 = 10;
const PREVIEW_CACHE: usize = 64;
/// Delay between hiding the picker and pasting, so focus is back on the target.
const PASTE_DELAY_MS: u64 = 120;

#[derive(Clone)]
enum Preview {
    Image(gdk::Texture),
    Text(String),
    Failed,
}

#[derive(Clone)]
struct Row {
    root: gtk::Box,
    num: gtk::Label,
    mark: gtk::Label,
    thumb: gtk::DrawingArea,
    thumb_image: Rc<RefCell<Option<Pixbuf>>>,
    text: gtk::Label,
}

impl Row {
    fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let num = gtk::Label::builder()
            .width_chars(3)
            .xalign(1.0)
            .css_classes(["num"])
            .build();
        let mark = gtk::Label::builder()
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .css_classes(["mark", "empty"])
            .build();
        let mark_slot = gtk::Box::builder().width_request(26).build();
        mark_slot.append(&mark);
        let thumb = gtk::DrawingArea::builder()
            .content_width(THUMB_W)
            .content_height(THUMB_H)
            .valign(gtk::Align::Center)
            .css_classes(["thumb"])
            .build();
        let thumb_image: Rc<RefCell<Option<Pixbuf>>> = Rc::default();
        let image = thumb_image.clone();
        thumb.set_draw_func(move |_, cr, w, h| draw_thumb(cr, w, h, image.borrow().as_ref()));
        let text = gtk::Label::builder()
            .hexpand(true)
            .xalign(0.0)
            .single_line_mode(true)
            .ellipsize(pango::EllipsizeMode::End)
            .css_classes(["text"])
            .build();
        root.append(&num);
        root.append(&mark_slot);
        root.append(&thumb);
        root.append(&text);
        Row {
            root,
            num,
            mark,
            thumb,
            thumb_image,
            text,
        }
    }

    fn set_thumb(&self, image: Option<&Pixbuf>) {
        self.thumb_image.replace(image.cloned());
        self.thumb.queue_draw();
    }

    fn set_mark(&self, number: Option<usize>) {
        match number {
            Some(n) => {
                self.mark.set_text(&n.to_string());
                self.mark.remove_css_class("empty");
            }
            None => {
                self.mark.set_text("");
                self.mark.add_css_class("empty");
            }
        }
    }
}

#[derive(Default)]
struct State {
    entries: Vec<Entry>,
    /// Entry id -> index in `entries` (= position in history, 0 = newest).
    index: HashMap<String, usize>,
    /// Entry ids currently in the list model (after filtering), mirroring it.
    model_ids: Vec<String>,
    marks: Marks,
    mode: Option<Mode>,
    /// Rows currently bound, by entry id.
    bound: HashMap<String, Row>,
    thumbs: HashMap<String, Pixbuf>,
    thumbs_pending: HashSet<String>,
    previews: HashMap<String, Preview>,
    /// The entry id the preview pane should show.
    preview_for: Option<String>,
}

pub struct Picker {
    window: gtk::ApplicationWindow,
    model: gtk::StringList,
    selection: gtk::SingleSelection,
    list: gtk::ListView,
    search: gtk::Entry,
    preview_stack: gtk::Stack,
    preview_image: gtk::Picture,
    preview_text: gtk::Label,
    preview_meta: gtk::Label,
    body_stack: gtk::Stack,
    state: RefCell<State>,
}

impl Picker {
    pub fn new(app: &gtk::Application) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .default_width(WIDTH)
            .default_height(HEIGHT)
            .decorated(false)
            .css_classes(["picker"])
            .build();
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.set_namespace(Some("cliphist-picker"));

        let model = gtk::StringList::new(&[]);
        let selection = gtk::SingleSelection::builder()
            .model(&model)
            .autoselect(true)
            .can_unselect(false)
            .build();
        let factory = gtk::SignalListItemFactory::new();
        let list = gtk::ListView::builder()
            .model(&selection)
            .factory(&factory)
            .single_click_activate(false)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .hexpand(true)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .css_classes(["list"])
            .build();

        let empty = gtk::Label::builder()
            .label("History is empty")
            .hexpand(true)
            .vexpand(true)
            .css_classes(["empty-history"])
            .build();
        let body_stack = gtk::Stack::new();
        body_stack.add_named(&scroller, Some("list"));
        body_stack.add_named(&empty, Some("empty"));

        let preview_image = gtk::Picture::builder()
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Contain)
            .margin_start(12)
            .margin_end(12)
            .margin_top(12)
            .margin_bottom(4)
            .build();
        let preview_text = gtk::Label::builder()
            .xalign(0.0)
            .yalign(0.0)
            .wrap(true)
            .wrap_mode(pango::WrapMode::WordChar)
            .selectable(false)
            .css_classes(["preview-text"])
            .build();
        let text_scroller = gtk::ScrolledWindow::builder()
            .child(&preview_text)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let preview_stack = gtk::Stack::builder().vexpand(true).build();
        preview_stack.add_named(&preview_image, Some("image"));
        preview_stack.add_named(&text_scroller, Some("text"));
        preview_stack.add_named(&gtk::Box::new(gtk::Orientation::Vertical, 0), Some("none"));
        let preview_meta = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::End)
            .css_classes(["preview-meta"])
            .build();
        let preview = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(PREVIEW_WIDTH)
            .css_classes(["preview"])
            .build();
        preview.append(&preview_stack);
        preview.append(&preview_meta);

        // A Paned with a fixed position keeps the preview at PREVIEW_WIDTH no
        // matter how large the previewed image's natural size is.
        let body = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&body_stack)
            .end_child(&preview)
            .position(WIDTH - 2 * FRAME_BORDER - SEPARATOR - PREVIEW_WIDTH)
            .resize_start_child(true)
            .resize_end_child(false)
            .shrink_start_child(false)
            .shrink_end_child(true)
            .wide_handle(false)
            .vexpand(true)
            .build();

        let search = gtk::Entry::builder()
            .placeholder_text("Search…")
            .visible(false)
            .css_classes(["search"])
            .build();

        let frame = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["frame"])
            .overflow(gtk::Overflow::Hidden)
            .build();
        frame.append(&search);
        frame.append(&body);
        window.set_child(Some(&frame));

        let picker = Rc::new(Picker {
            window,
            model,
            selection,
            list,
            search,
            preview_stack,
            preview_image,
            preview_text,
            preview_meta,
            body_stack,
            state: RefCell::new(State::default()),
        });
        picker.connect(&factory);
        picker
    }

    fn connect(self: &Rc<Self>, factory: &gtk::SignalListItemFactory) {
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let row = Row::new();
            item.set_child(Some(&row.root));
            unsafe { item.set_data("row", row) };
        });
        let weak = Rc::downgrade(self);
        factory.connect_bind(move |_, item| {
            let (Some(p), Some((id, row))) = (weak.upgrade(), item_row(item)) else {
                return;
            };
            p.bind_row(id, row);
        });
        let weak = Rc::downgrade(self);
        factory.connect_unbind(move |_, item| {
            let (Some(p), Some((id, row))) = (weak.upgrade(), item_row(item)) else {
                return;
            };
            let mut st = p.state.borrow_mut();
            if st.bound.get(&id).is_some_and(|r| r.root == row.root) {
                st.bound.remove(&id);
            }
        });

        let weak = Rc::downgrade(self);
        self.selection.connect_selected_notify(move |_| {
            if let Some(p) = weak.upgrade() {
                p.update_preview();
            }
        });

        let weak = Rc::downgrade(self);
        self.list.connect_activate(move |_, pos| {
            if let Some(p) = weak.upgrade() {
                p.selection.set_selected(pos);
                p.perform(Action::Copy);
            }
        });

        let weak = Rc::downgrade(self);
        self.search.connect_changed(move |entry| {
            if let Some(p) = weak.upgrade() {
                p.refilter(&entry.text());
            }
        });

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, _keyval, keycode, modifiers| {
            let Some(p) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let mode = p.mode();
            let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
            match keys::action(mode, keycode, shift) {
                Some(action) => {
                    p.perform(action);
                    glib::Propagation::Stop
                }
                None if mode == Mode::Normal => glib::Propagation::Stop,
                None => glib::Propagation::Proceed,
            }
        });
        self.window.add_controller(keys);
    }

    fn mode(&self) -> Mode {
        self.state.borrow().mode.unwrap_or(Mode::Normal)
    }

    pub fn toggle(self: &Rc<Self>) {
        if self.window.is_visible() {
            self.hide();
        } else {
            self.show();
        }
    }

    pub fn show(self: &Rc<Self>) {
        if self.window.is_visible() {
            return;
        }
        let t0 = std::time::Instant::now();
        let entries = history::list().unwrap_or_else(|e| {
            eprintln!("cliphist-picker: {e:#}");
            Vec::new()
        });
        {
            let mut st = self.state.borrow_mut();
            st.index = entries
                .iter()
                .enumerate()
                .map(|(i, e)| (e.id.clone(), i))
                .collect();
            st.entries = entries;
            st.marks.clear();
            st.mode = Some(Mode::Normal);
            st.preview_for = None;
        }
        self.search.set_visible(false);
        if self.search.text().is_empty() {
            self.refilter("");
        } else {
            self.search.set_text(""); // refilters via `changed`
        }
        self.refresh_rows();
        let t1 = t0.elapsed();
        self.window.set_visible(true);
        self.list.grab_focus();
        if std::env::var_os("CLIPHIST_PICKER_DEBUG").is_some() {
            eprintln!("show: prepare {t1:?}, total {:?}", t0.elapsed());
            self.window.add_tick_callback(move |_, _| {
                eprintln!("show: first frame after {:?}", t0.elapsed());
                glib::ControlFlow::Break
            });
        }
    }

    /// Do the expensive first-time work (surface, GL, fonts, CSS, list rows,
    /// thumbnails) at startup, so the first SUPER+SHIFT+V is as fast as later
    /// ones. The window is shown fully transparent for one frame, then hidden.
    pub fn warm_up(self: &Rc<Self>) {
        self.window.set_opacity(0.0);
        self.window.set_keyboard_mode(KeyboardMode::None);
        self.show();
        let weak = Rc::downgrade(self);
        self.window.add_tick_callback(move |_, _| {
            if let Some(p) = weak.upgrade() {
                // Second frame: rows are laid out and bound by now.
                glib::idle_add_local_once(move || {
                    p.hide();
                    p.window.set_opacity(1.0);
                    p.window.set_keyboard_mode(KeyboardMode::Exclusive);
                });
            }
            glib::ControlFlow::Break
        });
    }

    pub fn hide(&self) {
        self.window.set_visible(false);
    }

    fn refilter(self: &Rc<Self>, query: &str) {
        let (pos, removed, added, n, empty) = {
            let mut st = self.state.borrow_mut();
            let ids: Vec<String> = history::filter(&st.entries, query)
                .into_iter()
                .map(|i| st.entries[i].id.clone())
                .collect();
            let (pos, removed, range) = diff::splice(&st.model_ids, &ids);
            let added = ids[range].to_vec();
            let n = ids.len();
            st.model_ids = ids;
            (pos, removed, added, n, st.entries.is_empty())
        };
        if removed > 0 || !added.is_empty() {
            let refs: Vec<&str> = added.iter().map(String::as_str).collect();
            self.model.splice(pos as u32, removed as u32, &refs);
        }
        self.body_stack
            .set_visible_child_name(if empty { "empty" } else { "list" });
        if n > 0 {
            self.select(0);
        }
        self.update_preview();
    }

    fn select(&self, pos: u32) {
        self.selection.set_selected(pos);
        self.list.scroll_to(pos, gtk::ListScrollFlags::NONE, None);
    }

    fn current(&self) -> Option<usize> {
        let pos = self.selection.selected();
        let st = self.state.borrow();
        st.model_ids
            .get(pos as usize)
            .and_then(|id| st.index.get(id))
            .copied()
    }

    fn move_by(&self, delta: i64) {
        let n = self.model.n_items() as i64;
        if n == 0 {
            return;
        }
        let pos = (self.selection.selected() as i64 + delta).clamp(0, n - 1);
        self.select(pos as u32);
    }

    fn perform(self: &Rc<Self>, action: Action) {
        match action {
            Action::Down => self.move_by(1),
            Action::Up => self.move_by(-1),
            Action::PageDown => self.move_by(PAGE),
            Action::PageUp => self.move_by(-PAGE),
            Action::First => self.move_by(i64::MIN / 2),
            Action::Last => self.move_by(i64::MAX / 2),
            Action::ToggleMark => {
                let Some(idx) = self.current() else { return };
                self.state.borrow_mut().marks.toggle(idx);
                self.refresh_rows();
                self.move_by(1);
            }
            Action::Copy => {
                let Some(idx) = self.current() else { return };
                let line = self.state.borrow().entries[idx].line.clone();
                self.hide();
                if let Err(e) = history::copy(&line) {
                    eprintln!("cliphist-picker: copy: {e:#}");
                }
            }
            Action::Paste => {
                let lines: Vec<String> = {
                    let st = self.state.borrow();
                    let order: Vec<usize> = if st.marks.len() > 0 {
                        st.marks.order().to_vec()
                    } else {
                        self.current().into_iter().collect()
                    };
                    order.iter().map(|&i| st.entries[i].line.clone()).collect()
                };
                if lines.is_empty() {
                    return;
                }
                self.hide();
                glib::timeout_add_local_once(
                    std::time::Duration::from_millis(PASTE_DELAY_MS),
                    move || {
                        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
                        if let Err(e) = history::paste(&refs) {
                            eprintln!("cliphist-picker: paste: {e:#}");
                        }
                    },
                );
            }
            Action::StartSearch => {
                self.state.borrow_mut().mode = Some(Mode::Search);
                self.search.set_visible(true);
                self.search.grab_focus();
            }
            Action::EndSearch => {
                self.state.borrow_mut().mode = Some(Mode::Normal);
                self.search.set_visible(false);
                self.list.grab_focus();
            }
            Action::Close => self.hide(),
        }
    }

    /// Update numbers and marks of bound rows. Rows that a splice didn't touch
    /// keep their widgets, but their history position may have shifted.
    fn refresh_rows(&self) {
        {
            let st = self.state.borrow();
            for (id, row) in &st.bound {
                if let Some(&idx) = st.index.get(id) {
                    row.num.set_text(&(idx + 1).to_string());
                    row.set_mark(st.marks.number(idx));
                }
            }
        }
    }

    fn bind_row(self: &Rc<Self>, id: String, row: Row) {
        let load = {
            let mut st = self.state.borrow_mut();
            let Some(&idx) = st.index.get(&id) else {
                return;
            };
            let entry = st.entries[idx].clone();
            row.num.set_text(&(idx + 1).to_string());
            row.set_mark(st.marks.number(idx));
            row.text.set_text(&entry.label());
            let image = entry.is_image();
            row.thumb.set_visible(image);
            if image {
                row.text.add_css_class("image");
            } else {
                row.text.remove_css_class("image");
            }
            let mut load = None;
            if image {
                match st.thumbs.get(&entry.id) {
                    Some(image) => row.set_thumb(Some(image)),
                    None => {
                        row.set_thumb(None);
                        if st.thumbs_pending.insert(entry.id.clone()) {
                            load = Some(entry);
                        }
                    }
                }
            }
            st.bound.insert(id, row);
            load
        };
        if let Some(entry) = load {
            self.load_thumb(entry);
        }
    }

    fn load_thumb(self: &Rc<Self>, entry: Entry) {
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let line = entry.line.clone();
            let res =
                gio::spawn_blocking(move || thumbs::load_thumb(&line, THUMB_LOAD, THUMB_LOAD))
                    .await;
            let Some(p) = weak.upgrade() else { return };
            let mut st = p.state.borrow_mut();
            // Leave failures in `thumbs_pending` so they aren't retried on every bind.
            let Ok(Ok(raw)) = res else { return };
            let image = raw.into_pixbuf();
            st.thumbs_pending.remove(&entry.id);
            if let Some(row) = st.bound.get(&entry.id) {
                row.set_thumb(Some(&image));
            }
            st.thumbs.insert(entry.id, image);
        });
    }

    fn update_preview(self: &Rc<Self>) {
        let Some(idx) = self.current() else {
            self.preview_stack.set_visible_child_name("none");
            self.preview_meta.set_text("");
            return;
        };
        let (entry, cached) = {
            let mut st = self.state.borrow_mut();
            let entry = st.entries[idx].clone();
            st.preview_for = Some(entry.id.clone());
            let cached = st.previews.get(&entry.id).cloned();
            (entry, cached)
        };
        if let Some(preview) = cached {
            self.show_preview(&entry, &preview);
            return;
        }
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let line = entry.line.clone();
            let image = entry.is_image();
            let res = gio::spawn_blocking(move || {
                if image {
                    thumbs::load_image(&line, PREVIEW_LOAD_W, PREVIEW_LOAD_H).map(Preview::Image)
                } else {
                    thumbs::load_text(&line).map(Preview::Text)
                }
            })
            .await;
            let Some(p) = weak.upgrade() else { return };
            let preview = match res {
                Ok(Ok(preview)) => preview,
                _ => Preview::Failed,
            };
            let current = {
                let mut st = p.state.borrow_mut();
                if st.previews.len() >= PREVIEW_CACHE {
                    st.previews.clear();
                }
                st.previews.insert(entry.id.clone(), preview.clone());
                st.preview_for.as_deref() == Some(entry.id.as_str())
            };
            if current {
                p.show_preview(&entry, &preview);
            }
        });
    }

    fn show_preview(&self, entry: &Entry, preview: &Preview) {
        match preview {
            Preview::Image(tex) => {
                self.preview_image.set_paintable(Some(tex));
                self.preview_stack.set_visible_child_name("image");
            }
            Preview::Text(text) => {
                self.preview_text.set_text(text);
                self.preview_stack.set_visible_child_name("text");
            }
            Preview::Failed => {
                self.preview_text.set_text("(failed to read the entry)");
                self.preview_stack.set_visible_child_name("text");
            }
        }
        let meta = match (&entry.kind, preview) {
            (Kind::Image { format, dims, size }, _) => format!("{format} · {dims} · {size}"),
            (Kind::Binary { size }, _) => format!("binary · {size}"),
            (Kind::Text, Preview::Text(t)) => {
                let lines = t.lines().count().max(1);
                format!("{} chars · {} lines", t.chars().count(), lines)
            }
            _ => String::new(),
        };
        self.preview_meta.set_text(&meta);
    }
}

fn item_row(item: &glib::Object) -> Option<(String, Row)> {
    let item = item.downcast_ref::<gtk::ListItem>()?;
    let id = item
        .item()?
        .downcast::<gtk::StringObject>()
        .ok()?
        .string()
        .to_string();
    let row = unsafe { item.data::<Row>("row")?.as_ref().clone() };
    Some((id, row))
}

/// Run the daemon: build the hidden window and react to commands from `rx`.
pub fn run(rx: async_channel::Receiver<String>, show_now: bool) -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("io.github.komparator147477.CliphistPicker")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| {
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_application_prefer_dark_theme(true);
        }
        let display = gdk::Display::default().expect("no display");
        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        // Optional overrides, e.g. a translucent background (see README).
        let user_css = user_style_path();
        if user_css.exists() {
            let css = gtk::CssProvider::new();
            css.load_from_path(&user_css);
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
        }

        std::mem::forget(app.hold());
        let picker = Picker::new(app);
        let rx = rx.clone();
        let p = picker.clone();
        glib::spawn_future_local(async move {
            while let Ok(cmd) = rx.recv().await {
                match cmd.trim() {
                    "toggle" => p.toggle(),
                    "show" => p.show(),
                    "hide" => p.hide(),
                    other => eprintln!("cliphist-picker: unknown command {other:?}"),
                }
            }
        });
        if show_now {
            picker.show();
        } else {
            picker.warm_up();
        }
    });
    app.run_with_args::<&str>(&[])
}

/// `$XDG_CONFIG_HOME/cliphist-picker/style.css`, loaded after the built-in theme.
fn user_style_path() -> std::path::PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_default();
    config.join("cliphist-picker/style.css")
}

/// Paint `image` scaled to cover the `w`×`h` thumbnail, centered and clipped
/// to rounded corners.
fn draw_thumb(cr: &gtk::cairo::Context, w: i32, h: i32, image: Option<&Pixbuf>) {
    let (w, h) = (f64::from(w), f64::from(h));
    let r = 4.0_f64.min(w / 2.0).min(h / 2.0);
    let quarter = std::f64::consts::FRAC_PI_2;
    cr.new_sub_path();
    cr.arc(w - r, r, r, -quarter, 0.0);
    cr.arc(w - r, h - r, r, 0.0, quarter);
    cr.arc(r, h - r, r, quarter, 2.0 * quarter);
    cr.arc(r, r, r, 2.0 * quarter, 3.0 * quarter);
    cr.close_path();
    cr.clip();
    let Some(image) = image else { return };
    let (iw, ih) = (f64::from(image.width()), f64::from(image.height()));
    let scale = (w / iw).max(h / ih);
    cr.translate((w - iw * scale) / 2.0, (h - ih * scale) / 2.0);
    cr.scale(scale, scale);
    #[allow(deprecated)]
    cr.set_source_pixbuf(image, 0.0, 0.0);
    cr.source().set_filter(gtk::cairo::Filter::Good);
    let _ = cr.paint();
}
