use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::store::{self, Key};
use crate::vpk::{self, Entry};
use crate::wav;

#[derive(PartialEq, Eq, Clone, Copy)]
enum Filter {
    All,
    Modified,
    Original,
}

pub struct App {
    tf_dir: Option<PathBuf>,
    dir_text: String,
    entries: Vec<Entry>,
    filtered: Vec<usize>,
    selected: HashSet<usize>,
    anchor: Option<usize>,
    backup: HashMap<Key, Vec<u8>>,
    search: String,
    filter: Filter,
    gain: f32,
    replace_path: Option<PathBuf>,
    log: Vec<String>,
    writable: bool,
    load_error: Option<String>,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let detected = detect_tf_dir();
        let mut app = Self {
            tf_dir: None,
            dir_text: detected
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            entries: Vec::new(),
            filtered: Vec::new(),
            selected: HashSet::new(),
            anchor: None,
            backup: HashMap::new(),
            search: String::new(),
            filter: Filter::All,
            gain: 1.5,
            replace_path: None,
            log: Vec::new(),
            writable: false,
            load_error: None,
        };
        if let Some(dir) = detected {
            app.load(dir);
        } else {
            app.push("TF2 install not found - paste the tf folder path and press Load");
        }
        app
    }

    fn push(&mut self, msg: impl Into<String>) {
        self.log.push(msg.into());
        if self.log.len() > 500 {
            self.log.remove(0);
        }
    }

    fn load(&mut self, dir: PathBuf) {
        self.entries.clear();
        self.selected.clear();
        self.load_error = None;
        for pack in [vpk::PACK_MISC, vpk::PACK_VO] {
            match vpk::index_pack(&dir, pack) {
                Ok(mut list) => self.entries.append(&mut list),
                Err(e) => self.load_error = Some(format!("{}: {}", dir.display(), e)),
            }
        }
        self.entries.sort_by(|a, b| a.path.cmp(&b.path));
        self.backup = store::load(&dir);
        {
            let n = store::import_legacy(&dir, &mut self.backup);
            if n > 0 {
                let _ = store::save(&dir, &self.backup);
                self.push(format!("recovered {} original waves from python backups", n));
            }
        }
        self.tf_dir = Some(dir.clone());
        self.writable = vpk::can_write(&dir, vpk::PACK_MISC);
        self.push(format!(
            "loaded {} sounds | {} already modified",
            self.entries.len(),
            self.backup.len()
        ));
        self.refilter();
    }

    fn refilter(&mut self) {
        self.anchor = None;
        let q = self.search.to_lowercase();
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if !q.is_empty() && !e.path.to_lowercase().contains(&q) {
                    return false;
                }
                let modified = self.backup.contains_key(&e.key());
                match self.filter {
                    Filter::All => true,
                    Filter::Modified => modified,
                    Filter::Original => !modified,
                }
            })
            .map(|(i, _)| i)
            .collect();
    }

    fn selected_entries(&self) -> Vec<Entry> {
        let mut v: Vec<Entry> = self
            .selected
            .iter()
            .filter_map(|i| self.entries.get(*i).cloned())
            .collect();
        v.sort_by(|a, b| a.path.cmp(&b.path));
        v
    }

    fn apply_all(&mut self, label: &str, mut f: impl FnMut(&[u8]) -> Option<Vec<u8>>) {
        let Some(dir) = self.tf_dir.clone() else {
            self.push("no tf folder loaded");
            return;
        };
        if !vpk::can_write(&dir, vpk::PACK_MISC) {
            self.push("cannot write: close Team Fortress 2 first");
            self.writable = false;
            return;
        }
        self.writable = true;
        let targets = self.selected_entries();
        if targets.is_empty() {
            self.push("nothing selected");
            return;
        }
        let (mut ok, mut fail, mut skipped) = (0usize, 0usize, 0usize);
        for e in &targets {
            let current = match vpk::read_entry(&dir, e) {
                Ok(d) => d,
                Err(err) => {
                    self.push(format!("read {} failed: {}", e.path, err));
                    fail += 1;
                    continue;
                }
            };
            // always transform from the original bytes when we have them, so gain
            // does not stack and a second replace starts from a clean slate
            let baseline = self
                .backup
                .get(&e.key())
                .cloned()
                .unwrap_or_else(|| current.clone());
            let new = match f(&baseline) {
                Some(n) => n,
                None => {
                    skipped += 1;
                    self.push(format!("{}: unsupported format, skipped", e.path));
                    continue;
                }
            };
            if new.len() != e.len as usize {
                self.push(format!("{}: length changed, skipped", e.path));
                fail += 1;
                continue;
            }
            // durability: the original must be on disk before the game file changes
            self.backup.entry(e.key()).or_insert_with(|| baseline.clone());
            if let Err(err) = store::save(&dir, &self.backup) {
                self.push(format!("backup save failed, aborting: {}", err));
                fail += 1;
                break;
            }
            match vpk::write_entry(&dir, e, &new) {
                Ok(_) => match vpk::read_entry(&dir, e) {
                    Ok(check) if check == new => ok += 1,
                    Ok(_) => {
                        self.push(format!("{}: write did not stick, skipped", e.path));
                        fail += 1;
                    }
                    Err(err) => {
                        self.push(format!("{}: verify failed: {}", e.path, err));
                        fail += 1;
                    }
                },
                Err(err) => {
                    self.push(format!("write {} failed: {}", e.path, err));
                    fail += 1;
                }
            }
        }
        self.push(format!(
            "{}: {} ok, {} failed, {} skipped",
            label, ok, fail, skipped
        ));
        self.refilter();
    }

    fn restore_selected(&mut self) {
        let Some(dir) = self.tf_dir.clone() else { return };
        if !vpk::can_write(&dir, vpk::PACK_MISC) {
            self.push("cannot write: close Team Fortress 2 first");
            return;
        }
        let targets = self.selected_entries();
        let (mut ok, mut fail) = (0, 0);
        for e in &targets {
            if let Some(orig) = self.backup.remove(&e.key()) {
                if vpk::write_entry(&dir, e, &orig).is_ok() {
                    ok += 1;
                } else {
                    self.backup.insert(e.key(), orig);
                    fail += 1;
                }
            }
        }
        let _ = store::save(&dir, &self.backup);
        self.push(format!("restored {} files, {} failed", ok, fail));
        self.gain = 1.0;
        self.refilter();
    }

    fn restore_all(&mut self) {
        let Some(dir) = self.tf_dir.clone() else { return };
        if !vpk::can_write(&dir, vpk::PACK_MISC) {
            self.push("cannot write: close Team Fortress 2 first");
            return;
        }
        let keys: Vec<Key> = self.backup.keys().cloned().collect();
        let mut ok = 0;
        for k in keys {
            let e = Entry {
                path: String::new(),
                pack: k.0,
                arch: k.1,
                off: k.2,
                len: k.3,
            };
            if let Some(orig) = self.backup.get(&k) {
                if vpk::write_entry(&dir, &e, orig).is_ok() {
                    self.backup.remove(&k);
                    ok += 1;
                }
            }
        }
        let _ = store::save(&dir, &self.backup);
        self.push(format!("restored {} files total", ok));
        self.gain = 1.0;
        self.refilter();
    }

    fn db_label(gain: f32) -> String {
        let db = 20.0 * gain.log10();
        format!("{:+.1} dB", db)
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("TF2 Sound Studio");
            ui.label(egui::RichText::new("Team Fortress 2").weak());
            ui.separator();
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.dir_text)
                    .desired_width(430.0)
                    .hint_text(r"C:\Program Files (x86)\Steam\steamapps\common\Team Fortress 2\tf"),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let dir = PathBuf::from(self.dir_text.trim());
                self.load(dir);
            }
            if ui.button("Load").clicked() {
                let dir = PathBuf::from(self.dir_text.trim());
                self.load(dir);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (txt, col) = if self.writable {
                    ("VPK writable", egui::Color32::from_rgb(120, 200, 140))
                } else {
                    ("VPK locked (TF2 open)", egui::Color32::from_rgb(220, 140, 120))
                };
                ui.label(egui::RichText::new(txt).color(col).small());
            });
        });
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Actions");
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(format!("{} selected", self.selected.len()))
                .color(egui::Color32::from_rgb(150, 160, 180)),
        );
        ui.separator();

        ui.label("Amplify");
        ui.add(
            egui::Slider::new(&mut self.gain, 0.1..=4.0)
                .fixed_decimals(2)
                .suffix("x"),
        );
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(Self::db_label(self.gain)).weak());
            if ui.button("Apply gain").clicked() {
                let g = self.gain;
                self.apply_all("gain", move |base| wav::gain(base, g).map(|(d, _, _)| d));
            }
        });

        ui.add_space(8.0);
        ui.label("Mute");
        ui.horizontal(|ui| {
            if ui.button("Silence selected").clicked() {
                self.apply_all("mute", |base| wav::silence(base));
            }
        });

        ui.add_space(8.0);
        ui.label("Replace");
        let pick = self
            .replace_path
            .as_ref()
            .map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string())
            .unwrap_or_else(|| "no file selected".into());
        ui.label(egui::RichText::new(pick).weak().small());
        if let Some(path) = self.replace_path.clone() {
            match std::fs::read(&path).ok().and_then(|d| wav::info(&d)) {
                Some(w) if wav::is_pcm16(&w) => {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} Hz | {} ch | {} bit | {:.2}s",
                            w.rate,
                            w.channels,
                            w.bits,
                            wav::duration(&std::fs::read(&path).unwrap_or_default())
                                .unwrap_or(0.0)
                        ))
                        .weak()
                        .small(),
                    );
                }
                Some(_) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 140, 120),
                        "unsupported wav (need 16-bit PCM)",
                    );
                }
                None => {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 140, 120),
                        "not a readable wav",
                    );
                }
            }
        }
        ui.horizontal(|ui| {
            if ui.button("Browse").clicked() {
                if let Some(p) = rfd_pick() {
                    let ok = std::fs::read(&p)
                        .ok()
                        .and_then(|d| wav::info(&d))
                        .map(|w| wav::is_pcm16(&w))
                        .unwrap_or(false);
                    if ok {
                        self.push(format!("loaded {}", p.display()));
                        self.replace_path = Some(p);
                    } else {
                        self.push(format!("{} is not a 16-bit PCM wav", p.display()));
                    }
                }
            }
            if ui.button("Replace selected").clicked() {
                if let Some(src) = self.replace_path.clone() {
                    match std::fs::read(&src) {
                        Ok(src_bytes) => {
                            self.apply_all("replace", move |base| wav::replace_fit(base, &src_bytes));
                        }
                        Err(e) => self.push(format!("read {} failed: {}", src.display(), e)),
                    }
                } else {
                    self.push("pick a .wav first");
                }
            }
        });

        ui.add_space(12.0);
        ui.separator();
        ui.label("Restore");
        ui.horizontal(|ui| {
            if ui.button("Restore selected").clicked() {
                self.restore_selected();
            }
            if ui.button("Restore all").clicked() {
                self.restore_all();
            }
        });

        ui.add_space(12.0);
        ui.separator();
        ui.label(egui::RichText::new("Log").weak().small());
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .max_height(180.0)
            .show(ui, |ui| {
                let start = self.log.len().saturating_sub(40);
                for line in &self.log[start..] {
                    ui.label(egui::RichText::new(line).monospace().small());
                }
            });
    }

    fn table(&mut self, ui: &mut egui::Ui) {
        let typing = ui.memory(|m| m.focused().is_some());
        if !typing && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) {
            for i in &self.filtered {
                self.selected.insert(*i);
            }
        }
        if !typing && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::D)) {
            self.selected.clear();
            self.anchor = None;
        }
        ui.horizontal(|ui| {
            ui.label("Search");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .desired_width(320.0)
                    .hint_text("revolver, footsteps, cart..."),
            );
            if resp.changed() {
                // stale selections must not survive a new search
                self.selected.clear();
                self.anchor = None;
                self.refilter();
            }
            egui::ComboBox::from_id_salt("filter")
                .selected_text(match self.filter {
                    Filter::All => "All",
                    Filter::Modified => "Modified",
                    Filter::Original => "Original",
                })
                .show_ui(ui, |ui| {
                    for f in [Filter::All, Filter::Modified, Filter::Original] {
                        let name = match f {
                            Filter::All => "All",
                            Filter::Modified => "Modified",
                            Filter::Original => "Original",
                                };
                        if ui.selectable_label(self.filter == f, name).clicked() {
                            if self.filter != f {
                                self.selected.clear();
                            }
                            self.filter = f;
                            self.refilter();
                        }
                    }
                });
            if ui.button("Select visible").clicked() {
                for i in &self.filtered {
                    self.selected.insert(*i);
                }
            }
            if ui.button("Clear").clicked() {
                self.selected.clear();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{} shown / {} total",
                        self.filtered.len(),
                        self.entries.len()
                    ))
                    .weak()
                    .small(),
                );
            });
        });
        if let Some(err) = &self.load_error {
            ui.colored_label(egui::Color32::from_rgb(230, 130, 120), err);
        }
        ui.label(
            egui::RichText::new(
                "click = select one · ctrl/alt+click = add or remove · shift+click = range · ctrl+a all · ctrl+d none",
            )
            .weak()
            .small(),
        );
        ui.separator();

        let row_h = 20.0;
        let total = self.filtered.len();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_h, total, |ui, range| {
                for row in range {
                    let idx = self.filtered[row];
                    let key = self.entries[idx].key();
                    let modified = self.backup.contains_key(&key);
                    let path = self.entries[idx].path.clone();
                    let len = self.entries[idx].len;
                    ui.horizontal(|ui| {
                        let mut sel = self.selected.contains(&idx);
                        if ui.checkbox(&mut sel, "").changed() {
                            if sel {
                                self.selected.insert(idx);
                            } else {
                                self.selected.remove(&idx);
                            }
                            self.anchor = Some(row);
                        }
                        let tag = if modified { "M " } else { "   " };
                        let color = if modified {
                            egui::Color32::from_rgb(200, 170, 120)
                        } else {
                            egui::Color32::from_rgb(190, 195, 205)
                        };
                        ui.label(egui::RichText::new(tag).monospace().color(color));
                        let short = path.strip_prefix("sound/").unwrap_or(&path);
                        let lbl = ui.add(
                            egui::Label::new(egui::RichText::new(short).monospace().color(color))
                                .selectable(false)
                                .sense(egui::Sense::click()),
                        );
                        if lbl.clicked() {
                            let mods = ui.input(|i| i.modifiers);
                            if mods.shift {
                                let a = self.anchor.unwrap_or(row);
                                let (lo, hi) = if a <= row { (a, row) } else { (row, a) };
                                if !mods.command {
                                    self.selected.clear();
                                }
                                for p in lo..=hi {
                                    self.selected.insert(self.filtered[p]);
                                }
                            } else if mods.command || mods.alt {
                                if self.selected.contains(&idx) {
                                    self.selected.remove(&idx);
                                } else {
                                    self.selected.insert(idx);
                                }
                                self.anchor = Some(row);
                            } else {
                                if self.selected.len() == 1 && self.selected.contains(&idx) {
                                    self.selected.clear();
                                } else {
                                    self.selected.clear();
                                    self.selected.insert(idx);
                                }
                                self.anchor = Some(row);
                            }
                        }
                        if lbl.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!("{:.1} kB", len as f32 / 1024.0))
                                    .weak()
                                    .small(),
                            );
                        });
                    });
                }
            });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(4.0);
        });
        egui::SidePanel::right("side")
            .resizable(false)
            .exact_width(320.0)
            .show(ctx, |ui| self.side_panel(ui));
        egui::CentralPanel::default().show(ctx, |ui| self.table(ui));
    }
}

fn rfd_pick() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Wave audio", &["wav"])
        .add_filter("All files", &["*"])
        .set_title("Choose a replacement .wav")
        .pick_file()
}

pub fn detect_tf_dir() -> Option<PathBuf> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let mut libs: Vec<PathBuf> = Vec::new();
    if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Valve\Steam") {
        if let Ok(steam) = key.get_value::<String, _>("SteamPath") {
            libs.push(PathBuf::from(steam.replace('/', "\\")));
        }
    }
    let mut all = libs.clone();
    for lib in &libs {
        let vdf = lib.join("steamapps").join("libraryfolders.vdf");
        if let Ok(txt) = std::fs::read_to_string(&vdf) {
            for line in txt.lines() {
                if let Some(pos) = line.find("\"path\"") {
                    let rest = &line[pos + 6..];
                    if let Some(a) = rest.find('"') {
                        if let Some(b) = rest[a + 1..].find('"') {
                            let p = rest[a + 1..a + 1 + b].replace("\\\\", "\\");
                            all.push(PathBuf::from(p));
                        }
                    }
                }
            }
        }
    }
    for lib in all {
        let tf = lib
            .join("steamapps")
            .join("common")
            .join("Team Fortress 2")
            .join("tf");
        if tf.join("tf2_sound_misc_dir.vpk").exists() {
            return Some(tf);
        }
    }
    None
}
