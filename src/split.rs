//! PDF splitting logic, independent of the GUI.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use lopdf::{Dictionary, Document, Object, ObjectId};

/// User-facing error (invalid input, unreadable file, write failure...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitError {
    Invalid(String),
    PasswordRequired,
    WrongPassword,
    Pdf(String),
    Io(String),
    Cancelled,
}

impl fmt::Display for SplitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SplitError::Invalid(msg) => write!(f, "{msg}"),
            SplitError::PasswordRequired => write!(f, "This PDF is password protected."),
            SplitError::WrongPassword => write!(f, "The password is incorrect."),
            SplitError::Pdf(msg) => write!(f, "Could not read the PDF: {msg}"),
            SplitError::Io(msg) => write!(f, "Could not write file: {msg}"),
            SplitError::Cancelled => write!(f, "Cancelled."),
        }
    }
}

impl std::error::Error for SplitError {}

fn invalid(msg: impl Into<String>) -> SplitError {
    SplitError::Invalid(msg.into())
}

/// A group of 0-based page indices that will become one output file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageGroup {
    pub pages: Vec<usize>,
    /// Human readable, 1-based label used in the file name, e.g. `1-3` or `5`.
    pub label: String,
}

impl PageGroup {
    fn new(pages: Vec<usize>) -> Self {
        let label = label_for(&pages);
        PageGroup { pages, label }
    }
}

fn label_for(pages: &[usize]) -> String {
    let (first, last) = (pages[0] + 1, pages[pages.len() - 1] + 1);
    if pages.len() == 1 {
        return first.to_string();
    }
    let contiguous = pages.windows(2).all(|w| w[1] == w[0] + 1);
    if contiguous {
        format!("{first}-{last}")
    } else {
        format!("{first}-{last}_sel")
    }
}

/// The ways a document can be split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    EveryPage,
    EveryN,
    EqualParts,
    Ranges,
    Extract,
}

impl Mode {
    pub const ALL: [Mode; 5] = [
        Mode::EveryPage,
        Mode::EveryN,
        Mode::EqualParts,
        Mode::Ranges,
        Mode::Extract,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Mode::EveryPage => "Every page",
            Mode::EveryN => "Every N pages",
            Mode::EqualParts => "Equal parts",
            Mode::Ranges => "Custom ranges",
            Mode::Extract => "Extract pages",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Mode::EveryPage => "Each page becomes its own PDF file.",
            Mode::EveryN => "Cut the document into chunks of N pages.",
            Mode::EqualParts => "Divide the document into N files of (nearly) equal size.",
            Mode::Ranges => "One file per range, e.g.  1-3, 5, 8-last",
            Mode::Extract => "Pick pages and merge them into a single PDF, e.g.  1, 4-6, 9",
        }
    }

    /// Compute the output groups for this mode and the user's input.
    pub fn groups(self, page_count: usize, input: &str) -> Result<Vec<PageGroup>, SplitError> {
        if page_count == 0 {
            return Err(invalid("The document has no pages."));
        }
        let number = || -> Result<usize, SplitError> {
            input
                .trim()
                .parse::<usize>()
                .map_err(|_| invalid("Please enter a whole number."))
        };
        match self {
            Mode::EveryPage => Ok(groups_every_n(page_count, 1)?),
            Mode::EveryN => groups_every_n(page_count, number()?),
            Mode::EqualParts => groups_into_parts(page_count, number()?),
            Mode::Ranges => Ok(parse_ranges(input, page_count)?
                .into_iter()
                .map(PageGroup::new)
                .collect()),
            Mode::Extract => {
                let pages: Vec<usize> = parse_ranges(input, page_count)?.concat();
                Ok(vec![PageGroup {
                    pages,
                    label: "extract".into(),
                }])
            }
        }
    }
}

/// Parse a range spec like `"1-3, 5, 8-"` into lists of 0-based page indices.
///
/// * `N`   - single page
/// * `A-B` - pages A..=B (reverse order allowed: `5-3`)
/// * `A-`  - page A to the last page
/// * `-B`  - first page to page B
/// * `last` / `end` may be used as a page number.
///
/// Ranges are separated by commas or semicolons; each becomes its own list.
pub fn parse_ranges(spec: &str, page_count: usize) -> Result<Vec<Vec<usize>>, SplitError> {
    let parts: Vec<&str> = spec
        .split([',', ';'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Err(invalid("Please enter at least one page or range."));
    }

    let page_num = |s: &str, part: &str| -> Result<Option<usize>, SplitError> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(None);
        }
        let n = if s.eq_ignore_ascii_case("last") || s.eq_ignore_ascii_case("end") {
            page_count
        } else {
            s.parse::<usize>()
                .map_err(|_| invalid(format!("Invalid range: '{part}'")))?
        };
        if n < 1 || n > page_count {
            return Err(invalid(format!(
                "Page {n} in '{part}' is out of range (1-{page_count})."
            )));
        }
        Ok(Some(n))
    };

    let mut result = Vec::with_capacity(parts.len());
    for part in parts {
        let (start, end) = match part.split_once('-') {
            Some((a, b)) => {
                if b.contains('-') {
                    return Err(invalid(format!("Invalid range: '{part}'")));
                }
                (
                    page_num(a, part)?.unwrap_or(1),
                    page_num(b, part)?.unwrap_or(page_count),
                )
            }
            None => {
                let n = page_num(part, part)?.expect("part is non-empty");
                (n, n)
            }
        };
        let pages: Vec<usize> = if end >= start {
            (start - 1..end).collect()
        } else {
            (end - 1..start).rev().collect()
        };
        result.push(pages);
    }
    Ok(result)
}

pub fn groups_every_n(page_count: usize, n: usize) -> Result<Vec<PageGroup>, SplitError> {
    if n < 1 {
        return Err(invalid("Pages per file must be at least 1."));
    }
    Ok((0..page_count)
        .step_by(n)
        .map(|start| PageGroup::new((start..(start + n).min(page_count)).collect()))
        .collect())
}

pub fn groups_into_parts(page_count: usize, parts: usize) -> Result<Vec<PageGroup>, SplitError> {
    if parts < 1 {
        return Err(invalid("Number of files must be at least 1."));
    }
    let parts = parts.min(page_count);
    let (base, extra) = (page_count / parts, page_count % parts);
    let mut start = 0;
    Ok((0..parts)
        .map(|i| {
            let size = base + usize::from(i < extra);
            let group = PageGroup::new((start..start + size).collect());
            start += size;
            group
        })
        .collect())
}

/// Make a string safe to use as a Windows file name stem.
pub fn safe_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c| c == ' ' || c == '.');
    if trimmed.is_empty() {
        "document".into()
    } else {
        trimmed.into()
    }
}

/// File name for a group, with page numbers zero-padded so files sort well.
pub fn output_name(prefix: &str, group: &PageGroup, page_count: usize) -> String {
    let width = page_count.to_string().len();
    let label = group
        .label
        .split('-')
        .map(|part| match part.parse::<usize>() {
            Ok(n) => format!("{n:0width$}"),
            Err(_) => match part.split_once('_') {
                Some((num, rest)) if num.parse::<usize>().is_ok() => {
                    format!("{:0width$}_{rest}", num.parse::<usize>().unwrap())
                }
                _ => part.to_string(),
            },
        })
        .collect::<Vec<_>>()
        .join("-");
    format!("{}_{label}.pdf", safe_stem(prefix))
}

/// A loaded, decrypted PDF ready for splitting.
pub struct Pdf {
    doc: Document,
    pages: Vec<ObjectId>,
}

impl Pdf {
    /// Open a PDF. For encrypted files pass the password (an empty user password
    /// is tried automatically).
    pub fn open(path: &Path, password: Option<&str>) -> Result<Pdf, SplitError> {
        guard(|| Self::open_inner(path, password))
    }

    fn open_inner(path: &Path, password: Option<&str>) -> Result<Pdf, SplitError> {
        let result = match password {
            Some(pw) => Document::load_with_password(path, pw),
            None => Document::load(path),
        };
        let doc = result.map_err(|err| match err {
            lopdf::Error::InvalidPassword => SplitError::WrongPassword,
            other => SplitError::Pdf(other.to_string()),
        })?;
        if doc.trailer.has(b"Encrypt") && doc.encryption_state.is_none() {
            return Err(if password.is_some() {
                SplitError::WrongPassword
            } else {
                SplitError::PasswordRequired
            });
        }
        let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
        if pages.is_empty() {
            return Err(SplitError::Pdf("the document has no pages".into()));
        }
        Ok(Pdf { doc, pages })
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Build a standalone document containing the given pages, in order.
    pub fn extract(&self, pages: &[usize]) -> Result<Document, SplitError> {
        guard(|| self.extract_inner(pages))
    }

    fn extract_inner(&self, pages: &[usize]) -> Result<Document, SplitError> {
        let src = &self.doc;
        let mut out = Document::with_version(src.version.clone());
        // Keep the original object ids for copied objects; new ids come after them.
        out.max_id = src.max_id;

        let pages_root = out.new_object_id();
        let mut copier = Copier::new(src, pages.iter().map(|&i| self.pages[i]).collect());
        let mut kids = Vec::with_capacity(pages.len());

        for &index in pages {
            let src_id = *self
                .pages
                .get(index)
                .ok_or_else(|| invalid(format!("Page {} does not exist.", index + 1)))?;
            let mut page = src
                .get_dictionary(src_id)
                .map_err(|e| SplitError::Pdf(e.to_string()))?
                .clone();
            // Our page tree is flat, so attributes inherited from parent nodes
            // must be copied onto the page itself.
            for key in [&b"Resources"[..], b"MediaBox", b"CropBox", b"Rotate"] {
                if !page.has(key)
                    && let Some(value) = inherited(src, &page, key)
                {
                    page.set(key.to_vec(), value);
                }
            }
            page.set("Parent", Object::Reference(pages_root));
            let page = copier.remap_dict(page);

            // A page used twice in one output needs its own object.
            let id = if copier.copied.insert(src_id) {
                src_id
            } else {
                out.new_object_id()
            };
            out.objects.insert(id, Object::Dictionary(page));
            kids.push(Object::Reference(id));
        }
        copier.drain_into(&mut out);

        let mut root = Dictionary::new();
        root.set("Type", Object::Name(b"Pages".to_vec()));
        root.set("Count", Object::Integer(kids.len() as i64));
        root.set("Kids", Object::Array(kids));
        out.objects.insert(pages_root, Object::Dictionary(root));

        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(pages_root));
        // Keep form fields so interactive forms on the kept pages still work.
        if let Ok(src_catalog) = src.catalog()
            && let Ok(acro) = src_catalog.get(b"AcroForm")
        {
            catalog.set("AcroForm", copier.remap(acro.clone()));
        }
        let catalog_id = out.add_object(Object::Dictionary(catalog));
        out.trailer.set("Root", Object::Reference(catalog_id));

        if let Ok(info) = src.trailer.get(b"Info") {
            let info = copier.remap(info.clone());
            out.trailer.set("Info", info);
        }
        copier.drain_into(&mut out);

        out.renumber_objects();
        Ok(out)
    }
}

/// Turn a panic inside the PDF library (on a malformed file) into an error
/// instead of crashing the application.
fn guard<T>(f: impl FnOnce() -> Result<T, SplitError>) -> Result<T, SplitError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| {
        Err(SplitError::Pdf(
            "the file is damaged or uses an unsupported feature".into(),
        ))
    })
}

/// Look up an inheritable page attribute in the page's ancestors.
fn inherited(doc: &Document, page: &Dictionary, key: &[u8]) -> Option<Object> {
    let mut node = page.get(b"Parent").and_then(Object::as_reference).ok();
    let mut seen = HashSet::new();
    while let Some(id) = node {
        if !seen.insert(id) {
            break; // cycle in a malformed file
        }
        let dict = doc.get_dictionary(id).ok()?;
        if let Ok(value) = dict.get(key) {
            return Some(value.clone());
        }
        node = dict.get(b"Parent").and_then(Object::as_reference).ok();
    }
    None
}

/// Copies objects reachable from the kept pages, dropping references to
/// pages that are not part of the output (and to the old page tree), so the
/// output doesn't silently contain the whole original document.
struct Copier<'a> {
    src: &'a Document,
    kept_pages: HashSet<ObjectId>,
    copied: HashSet<ObjectId>,
    queue: VecDeque<ObjectId>,
    visited: HashSet<ObjectId>,
    is_page_node: HashMap<ObjectId, bool>,
}

impl<'a> Copier<'a> {
    fn new(src: &'a Document, kept_pages: HashSet<ObjectId>) -> Self {
        Copier {
            src,
            kept_pages,
            copied: HashSet::new(),
            queue: VecDeque::new(),
            visited: HashSet::new(),
            is_page_node: HashMap::new(),
        }
    }

    fn page_node(&mut self, id: ObjectId) -> bool {
        let src = self.src;
        *self.is_page_node.entry(id).or_insert_with(|| {
            src.get_dictionary(id)
                .and_then(|d| d.get(b"Type"))
                .and_then(Object::as_name)
                .map(|t| t == b"Page" || t == b"Pages")
                .unwrap_or(false)
        })
    }

    fn remap(&mut self, obj: Object) -> Object {
        match obj {
            Object::Reference(id) => {
                if self.page_node(id) {
                    // Links to kept pages stay; everything else in the page tree is dropped.
                    if self.kept_pages.contains(&id) {
                        Object::Reference(id)
                    } else {
                        Object::Null
                    }
                } else if self.src.objects.contains_key(&id) {
                    if self.visited.insert(id) {
                        self.queue.push_back(id);
                    }
                    Object::Reference(id)
                } else {
                    Object::Null
                }
            }
            Object::Array(items) => {
                Object::Array(items.into_iter().map(|o| self.remap(o)).collect())
            }
            Object::Dictionary(dict) => Object::Dictionary(self.remap_dict(dict)),
            Object::Stream(mut stream) => {
                stream.dict = self.remap_dict(stream.dict);
                Object::Stream(stream)
            }
            other => other,
        }
    }

    fn remap_dict(&mut self, dict: Dictionary) -> Dictionary {
        let mut out = Dictionary::new();
        for (key, value) in dict.into_iter() {
            out.set(key, self.remap(value));
        }
        out
    }

    fn drain_into(&mut self, out: &mut Document) {
        while let Some(id) = self.queue.pop_front() {
            if let Some(obj) = self.src.objects.get(&id) {
                let obj = self.remap(obj.clone());
                out.objects.insert(id, obj);
            }
        }
    }
}

/// Progress information reported while writing.
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub files_done: usize,
    pub files_total: usize,
}

/// Write each group to `out_dir/<prefix>_<label>.pdf`. Returns the written paths.
pub fn write_groups(
    pdf: &Pdf,
    groups: &[PageGroup],
    out_dir: &Path,
    prefix: &str,
    overwrite: bool,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<Vec<PathBuf>, SplitError> {
    std::fs::create_dir_all(out_dir)
        .map_err(|e| SplitError::Io(format!("{}: {e}", out_dir.display())))?;
    let mut written = Vec::with_capacity(groups.len());
    for (i, group) in groups.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(SplitError::Cancelled);
        }
        let mut doc = pdf.extract(&group.pages)?;
        let target = unique_path(
            &out_dir.join(output_name(prefix, group, pdf.page_count())),
            overwrite,
        );
        let file = File::create(&target)
            .map_err(|e| SplitError::Io(format!("{}: {e}", target.display())))?;
        let mut writer = BufWriter::new(file);
        guard(|| {
            doc.save_to(&mut writer)
                .map_err(|e| SplitError::Io(format!("{}: {e}", target.display())))
        })?;
        written.push(target);
        progress(Progress {
            files_done: i + 1,
            files_total: groups.len(),
        });
    }
    Ok(written)
}

fn unique_path(path: &Path, overwrite: bool) -> PathBuf {
    if overwrite || !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2..)
        .map(|i| path.with_file_name(format!("{stem} ({i}){ext}")))
        .find(|p| !p.exists())
        .expect("infinite iterator")
}
