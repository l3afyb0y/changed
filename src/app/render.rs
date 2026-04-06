use crate::category::Category;
use crate::config::{Config, DiffMode, PathKind, RedactionMode, TrackedPackage, TrackedPath};
use crate::journal::{EventKind, JournalEvent};
use crate::scope::Scope;

use super::paths::AppPaths;

#[derive(Clone, Copy)]
pub struct CategoryFilters<'a> {
    include: &'a [Category],
    exclude: &'a [Category],
}

impl<'a> CategoryFilters<'a> {
    pub fn new(include: &'a [Category], exclude: &'a [Category]) -> Self {
        Self { include, exclude }
    }

    pub fn matches(self, category: Category) -> bool {
        let included = self.include.is_empty() || self.include.contains(&category);
        let excluded = self.exclude.contains(&category);
        included && !excluded
    }
}

pub fn render_init_summary(paths: &AppPaths, config: &Config, created: bool) -> String {
    let mut out = String::new();
    if created {
        out.push_str("Initialized changed.\n");
    } else {
        out.push_str("changed is already initialized.\n");
    }
    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!("Config: {}\n", paths.config_file().display()),
    );
    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!("State: {}\n", paths.state_home.display()),
    );
    let _ = std::fmt::Write::write_fmt(
        &mut out,
        format_args!(
            "Tracked paths: {} | tracked packages: {}\n",
            config.tracked_paths.len(),
            config.tracked_packages.len()
        ),
    );

    if !config.tracked_paths.is_empty() {
        out.push_str("Enabled categories:\n");
        for category in Category::ALL {
            let count = config
                .tracked_paths
                .iter()
                .filter(|entry| entry.category == category)
                .count();
            if count > 0 {
                let _ = std::fmt::Write::write_fmt(
                    &mut out,
                    format_args!("  - {} ({})\n", category, count),
                );
            }
        }
    }

    out
}

pub fn render_tracked(
    scoped_configs: &[(Scope, Config)],
    filters: CategoryFilters<'_>,
    path: Option<&str>,
    color: bool,
) -> String {
    let mut out = String::new();
    let palette = Palette::new(color);
    if scoped_configs.is_empty() {
        return String::from("Nothing is currently tracked for that filter.");
    }

    let mut wrote_any = false;
    for (scope, config) in scoped_configs {
        let filtered_paths: Vec<&TrackedPath> = config
            .tracked_paths
            .iter()
            .filter(|entry| filters.matches(entry.category))
            .filter(|entry| path.is_none_or(|wanted| entry.path.as_str() == wanted))
            .collect();

        let filtered_packages: Vec<&TrackedPackage> = config
            .tracked_packages
            .iter()
            .filter(|_| filters.matches(Category::Packages))
            .collect();

        if filtered_paths.is_empty() && filtered_packages.is_empty() {
            continue;
        }

        if wrote_any {
            out.push('\n');
        }
        wrote_any = true;
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{scope}:\n"));

        for current in Category::ALL {
            let section_paths: Vec<&TrackedPath> = filtered_paths
                .iter()
                .copied()
                .filter(|entry| entry.category == current)
                .collect();
            let section_packages: Vec<&TrackedPackage> = filtered_packages
                .iter()
                .copied()
                .filter(|_| current == Category::Packages)
                .collect();

            if section_paths.is_empty() && section_packages.is_empty() {
                continue;
            }

            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!("  {}:\n", palette.category(current.to_string())),
            );
            for entry in section_paths {
                let _ = std::fmt::Write::write_fmt(
                    &mut out,
                    format_args!(
                        "    - {} [{}; {}; {}]\n",
                        palette.path(&entry.path),
                        kind_label(entry.kind),
                        diff_mode_label(entry.diff_mode),
                        redaction_label(entry.redaction)
                    ),
                );
            }
            for pkg in section_packages {
                let _ = std::fmt::Write::write_fmt(
                    &mut out,
                    format_args!("    - {} {}\n", pkg.manager, pkg.package_name),
                );
            }
        }
    }

    if !wrote_any {
        return String::from("Nothing is currently tracked for that filter.");
    }

    out.trim_end().to_owned()
}

pub fn render_history(
    events: &[JournalEvent],
    clean: bool,
    limit: Option<usize>,
    color: bool,
) -> String {
    let mut sorted: Vec<&JournalEvent> = events.iter().collect();
    sorted.sort_by_key(|event| event.timestamp);
    let selected: Vec<&JournalEvent> = match limit {
        Some(limit) => sorted
            .into_iter()
            .rev()
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect(),
        None => sorted,
    };

    let mut out = String::from("# Changes\n\n");
    let palette = Palette::new(color);
    let mut current_date = None;
    for event in selected {
        let date = event.timestamp.date();
        if current_date != Some(date) {
            if current_date.is_some() {
                out.push('\n');
            }
            current_date = Some(date);
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!("## {}\n\n", palette.date(format_date(date))),
            );
        }

        if clean {
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!(
                    "- {} [{}/{}] {}: {}{}\n",
                    palette.time(format_time(event.timestamp.time())),
                    event.scope,
                    palette.category(event.category.to_string()),
                    palette.path(&event.path),
                    event.summary,
                    if event.diff.is_none() && event.kind == EventKind::Modified {
                        format!(" {}", palette.muted("[metadata-only]"))
                    } else {
                        String::new()
                    }
                ),
            );
        } else {
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!(
                    "### {}\n",
                    palette.time(format_time(event.timestamp.time()))
                ),
            );
            let _ = std::fmt::Write::write_fmt(&mut out, format_args!("Scope: {}\n", event.scope));
            let category_text = if event.category == Category::Packages {
                palette.undefined_category(event.category.to_string())
            } else {
                palette.category(event.category.to_string())
            };
            let _ =
                std::fmt::Write::write_fmt(&mut out, format_args!("Category: {category_text}\n"));
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!("{}\n", palette.path(&event.path)),
            );
            let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{}\n", event.summary));
            if let Some(diff) = &event.diff {
                for line in diff.lines() {
                    let styled = if line.starts_with("(+)") {
                        palette.add(line)
                    } else if line.starts_with("(-)") {
                        palette.remove(line)
                    } else {
                        line.to_owned()
                    };
                    let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{styled}\n"));
                }
            }
            out.push('\n');
        }
    }

    out.trim_end().to_owned()
}

struct Palette {
    enabled: bool,
}

impl Palette {
    fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    fn paint(&self, code: &str, text: impl AsRef<str>) -> String {
        let text = text.as_ref();
        if self.enabled {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    fn date(&self, text: impl AsRef<str>) -> String {
        self.paint("1;2", text)
    }

    fn time(&self, text: impl AsRef<str>) -> String {
        self.paint("36", text)
    }

    fn category(&self, text: impl AsRef<str>) -> String {
        self.paint("35", text)
    }

    fn path(&self, text: impl AsRef<str>) -> String {
        self.paint("34", text)
    }

    fn add(&self, text: impl AsRef<str>) -> String {
        self.paint("32", text)
    }

    fn remove(&self, text: impl AsRef<str>) -> String {
        self.paint("31", text)
    }

    fn muted(&self, text: impl AsRef<str>) -> String {
        self.paint("2", text)
    }

    fn undefined_category(&self, text: impl AsRef<str>) -> String {
        self.paint("33", text)
    }
}

fn diff_mode_label(mode: DiffMode) -> &'static str {
    match mode {
        DiffMode::MetadataOnly => "metadata-only",
        DiffMode::Unified => "unified diff",
    }
}

fn redaction_label(mode: RedactionMode) -> &'static str {
    match mode {
        RedactionMode::Off => "off",
        RedactionMode::Auto => "auto",
    }
}

fn kind_label(kind: PathKind) -> &'static str {
    match kind {
        PathKind::File => "file",
        PathKind::Directory => "dir",
    }
}

fn format_date(date: time::Date) -> String {
    format!(
        "{:02}/{:02}/{:02}",
        u8::from(date.month()),
        date.day(),
        date.year() % 100
    )
}

fn format_time(time: time::Time) -> String {
    let hour = time.hour();
    let period = if hour >= 12 { "pm" } else { "am" };
    let mut display_hour = hour % 12;
    if display_hour == 0 {
        display_hour = 12;
    }
    format!("{display_hour}:{:02}{period}", time.minute())
}
