use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use lopdf::{Dictionary, Document, Object, Stream};
use pdf_splitter::split::*;

/// Build a test PDF whose page `i` has MediaBox width `100 + i` and a unique
/// content stream. Pages live in a nested page tree; the MediaBox and
/// Resources are inherited from the intermediate nodes. Page 1 carries a link
/// annotation pointing to the last page.
fn make_pdf(dir: &Path, pages: usize) -> PathBuf {
    let mut doc = Document::with_version("1.7");
    let root_id = doc.new_object_id();
    let font_id = doc.add_object(Dictionary::from_iter(vec![
        ("Type", Object::Name(b"Font".to_vec())),
        ("Subtype", Object::Name(b"Type1".to_vec())),
        ("BaseFont", Object::Name(b"Helvetica".to_vec())),
    ]));
    let resources = Dictionary::from_iter(vec![(
        "Font",
        Object::Dictionary(Dictionary::from_iter(vec![(
            "F1",
            Object::Reference(font_id),
        )])),
    )]);

    let mut chunk_ids = Vec::new();
    let mut page_ids = Vec::new();
    for chunk in (0..pages).collect::<Vec<_>>().chunks(3) {
        let chunk_id = doc.new_object_id();
        let mut kids = Vec::new();
        for &i in chunk {
            let content = format!("BT /F1 12 Tf 10 50 Td (Page {}) Tj ET", i + 1);
            let content_id = doc.add_object(Stream::new(Dictionary::new(), content.into_bytes()));
            let mut page = Dictionary::from_iter(vec![
                ("Type", Object::Name(b"Page".to_vec())),
                ("Parent", Object::Reference(chunk_id)),
                ("Contents", Object::Reference(content_id)),
            ]);
            // Inherited MediaBox is on the intermediate node, so each page overrides with its own.
            page.set(
                "MediaBox",
                Object::Array(vec![
                    0.into(),
                    0.into(),
                    ((100 + i) as i64).into(),
                    100.into(),
                ]),
            );
            let id = doc.add_object(Object::Dictionary(page));
            kids.push(Object::Reference(id));
            page_ids.push(id);
        }
        let node = Dictionary::from_iter(vec![
            ("Type", Object::Name(b"Pages".to_vec())),
            ("Parent", Object::Reference(root_id)),
            ("Count", Object::Integer(chunk.len() as i64)),
            ("Kids", Object::Array(kids)),
            ("Resources", Object::Dictionary(resources.clone())),
        ]);
        doc.objects.insert(chunk_id, Object::Dictionary(node));
        chunk_ids.push(Object::Reference(chunk_id));
    }
    // Link from page 1 to the last page.
    let link = doc.add_object(Dictionary::from_iter(vec![
        ("Type", Object::Name(b"Annot".to_vec())),
        ("Subtype", Object::Name(b"Link".to_vec())),
        (
            "Rect",
            Object::Array(vec![0.into(), 0.into(), 10.into(), 10.into()]),
        ),
        (
            "Dest",
            Object::Array(vec![
                Object::Reference(*page_ids.last().unwrap()),
                Object::Name(b"Fit".to_vec()),
            ]),
        ),
    ]));
    doc.get_object_mut(page_ids[0])
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", Object::Array(vec![Object::Reference(link)]));

    doc.objects.insert(
        root_id,
        Object::Dictionary(Dictionary::from_iter(vec![
            ("Type", Object::Name(b"Pages".to_vec())),
            ("Count", Object::Integer(pages as i64)),
            ("Kids", Object::Array(chunk_ids)),
        ])),
    );
    let catalog = doc.add_object(Dictionary::from_iter(vec![
        ("Type", Object::Name(b"Catalog".to_vec())),
        ("Pages", Object::Reference(root_id)),
    ]));
    doc.trailer.set("Root", Object::Reference(catalog));
    let path = dir.join("sample.pdf");
    doc.save(&path).unwrap();
    path
}

/// Page markers (MediaBox width - 100) of a written file, in page order.
fn markers(path: &Path) -> Vec<i64> {
    let doc = Document::load(path).unwrap();
    doc.get_pages()
        .values()
        .map(|&id| {
            let page = doc.get_dictionary(id).unwrap();
            let mb = page.get(b"MediaBox").unwrap().as_array().unwrap();
            mb[2].as_i64().unwrap() - 100
        })
        .collect()
}

fn page_texts(path: &Path) -> Vec<String> {
    let doc = Document::load(path).unwrap();
    doc.objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .map(|s| String::from_utf8_lossy(&s.content).into_owned())
        .filter(|s| s.contains("(Page "))
        .collect()
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn parse_ranges_variants() {
    assert_eq!(
        parse_ranges("1-3, 5", 10).unwrap(),
        vec![vec![0, 1, 2], vec![4]]
    );
    assert_eq!(parse_ranges("8-", 10).unwrap(), vec![vec![7, 8, 9]]);
    assert_eq!(parse_ranges("-2", 10).unwrap(), vec![vec![0, 1]]);
    assert_eq!(
        parse_ranges("3-1; last", 10).unwrap(),
        vec![vec![2, 1, 0], vec![9]]
    );
    assert_eq!(parse_ranges("9-END", 10).unwrap(), vec![vec![8, 9]]);
    assert_eq!(parse_ranges(" 2 - 4 ,", 10).unwrap(), vec![vec![1, 2, 3]]);
    assert_eq!(parse_ranges("-", 3).unwrap(), vec![vec![0, 1, 2]]);
}

#[test]
fn parse_ranges_invalid() {
    for spec in ["", " , ", "abc", "0", "11", "1-20", "1--2", "1-2-3", "2.5"] {
        assert!(
            parse_ranges(spec, 10).is_err(),
            "{spec:?} should be rejected"
        );
    }
}

#[test]
fn group_modes() {
    let pages = |g: Vec<PageGroup>| g.into_iter().map(|g| g.pages).collect::<Vec<_>>();
    assert_eq!(Mode::EveryPage.groups(3, "").unwrap().len(), 3);
    assert_eq!(
        pages(Mode::EveryN.groups(5, "2").unwrap()),
        vec![vec![0, 1], vec![2, 3], vec![4]]
    );
    let parts: Vec<usize> = Mode::EqualParts
        .groups(10, "3")
        .unwrap()
        .iter()
        .map(|g| g.pages.len())
        .collect();
    assert_eq!(parts, vec![4, 3, 3]);
    assert_eq!(Mode::EqualParts.groups(2, "5").unwrap().len(), 2);
    assert!(Mode::EveryN.groups(5, "0").is_err());
    assert!(Mode::EveryN.groups(5, "x").is_err());
    let labels: Vec<String> = Mode::Ranges
        .groups(10, "1-3,5")
        .unwrap()
        .into_iter()
        .map(|g| g.label)
        .collect();
    assert_eq!(labels, vec!["1-3", "5"]);
    assert_eq!(
        pages(Mode::Extract.groups(10, "1,3-4").unwrap()),
        vec![vec![0, 2, 3]]
    );
}

#[test]
fn output_names() {
    let g = Mode::Ranges.groups(120, "2-4, 7, 9-8").unwrap();
    assert_eq!(output_name("Report", &g[0], 120), "Report_002-004.pdf");
    assert_eq!(output_name("Report", &g[1], 120), "Report_007.pdf");
    assert_eq!(output_name("Report", &g[2], 120), "Report_009-008_sel.pdf");
    assert_eq!(safe_stem("a:b/c?. "), "a_b_c_");
    assert_eq!(safe_stem(" .. "), "document");
}

#[test]
fn split_every_page() {
    let dir = tempfile::tempdir().unwrap();
    let src = make_pdf(dir.path(), 10);
    let pdf = Pdf::open(&src, None).unwrap();
    assert_eq!(pdf.page_count(), 10);

    let mut calls = Vec::new();
    let groups = Mode::EveryPage.groups(10, "").unwrap();
    let files = write_groups(
        &pdf,
        &groups,
        &dir.path().join("out"),
        "doc",
        false,
        &no_cancel(),
        |p| calls.push((p.files_done, p.files_total)),
    )
    .unwrap();
    assert_eq!(files.len(), 10);
    assert_eq!(files[0].file_name().unwrap(), "doc_01.pdf");
    assert_eq!(files[9].file_name().unwrap(), "doc_10.pdf");
    for (i, f) in files.iter().enumerate() {
        assert_eq!(markers(f), vec![i as i64]);
        // Only this page's content is in the file, not the whole document.
        assert_eq!(
            page_texts(f),
            vec![format!("BT /F1 12 Tf 10 50 Td (Page {}) Tj ET", i + 1)]
        );
    }
    assert_eq!(calls.last(), Some(&(10, 10)));

    // Inherited resources were copied onto the page.
    let doc = Document::load(&files[4]).unwrap();
    let page = doc
        .get_dictionary(*doc.get_pages().values().next().unwrap())
        .unwrap();
    assert!(page.has(b"Resources"));
}

#[test]
fn link_to_dropped_page_does_not_pull_in_whole_document() {
    let dir = tempfile::tempdir().unwrap();
    let src = make_pdf(dir.path(), 9);
    let pdf = Pdf::open(&src, None).unwrap();
    // Page 1 links to page 9; extracting only page 1 must not include page 9.
    let groups = Mode::Ranges.groups(9, "1").unwrap();
    let files = write_groups(&pdf, &groups, dir.path(), "x", false, &no_cancel(), |_| {}).unwrap();
    assert_eq!(page_texts(&files[0]).len(), 1);
    // But a link between two kept pages keeps working.
    let groups = Mode::Extract.groups(9, "1, 9").unwrap();
    let files = write_groups(&pdf, &groups, dir.path(), "x", false, &no_cancel(), |_| {}).unwrap();
    let doc = Document::load(&files[0]).unwrap();
    let pages: Vec<_> = doc.get_pages().into_values().collect();
    let annots = doc
        .get_dictionary(pages[0])
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap();
    let link = doc
        .get_dictionary(annots[0].as_reference().unwrap())
        .unwrap();
    let dest = link.get(b"Dest").unwrap().as_array().unwrap();
    assert_eq!(dest[0].as_reference().unwrap(), pages[1]);
}

#[test]
fn ranges_reordering_and_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let src = make_pdf(dir.path(), 10);
    let pdf = Pdf::open(&src, None).unwrap();

    let groups = Mode::Ranges.groups(10, "2-4, 10, 3-1").unwrap();
    let files = write_groups(&pdf, &groups, dir.path(), "x", false, &no_cancel(), |_| {}).unwrap();
    let names: Vec<_> = files
        .iter()
        .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["x_02-04.pdf", "x_10.pdf", "x_03-01_sel.pdf"]);
    assert_eq!(markers(&files[0]), vec![1, 2, 3]);
    assert_eq!(markers(&files[2]), vec![2, 1, 0]);

    let groups = Mode::Extract.groups(10, "5, 1-2, 5").unwrap();
    let files = write_groups(&pdf, &groups, dir.path(), "x", false, &no_cancel(), |_| {}).unwrap();
    assert_eq!(files[0].file_name().unwrap(), "x_extract.pdf");
    assert_eq!(markers(&files[0]), vec![4, 0, 1, 4]);
}

#[test]
fn never_overwrites_unless_asked() {
    let dir = tempfile::tempdir().unwrap();
    let src = make_pdf(dir.path(), 4);
    let pdf = Pdf::open(&src, None).unwrap();
    let groups = Mode::EqualParts.groups(4, "1").unwrap();
    let run = |overwrite| {
        write_groups(
            &pdf,
            &groups,
            dir.path(),
            "d",
            overwrite,
            &no_cancel(),
            |_| {},
        )
        .unwrap()
    };
    let a = run(false);
    let b = run(false);
    assert_eq!(a[0].file_name().unwrap(), "d_1-4.pdf");
    assert_eq!(b[0].file_name().unwrap(), "d_1-4 (2).pdf");
    assert_eq!(run(true)[0], a[0]);
}

#[test]
fn cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let src = make_pdf(dir.path(), 4);
    let pdf = Pdf::open(&src, None).unwrap();
    let groups = Mode::EveryPage.groups(4, "").unwrap();
    let cancel = AtomicBool::new(true);
    let result = write_groups(
        &pdf,
        &groups,
        &dir.path().join("out"),
        "d",
        false,
        &cancel,
        |_| {},
    );
    assert_eq!(result.unwrap_err(), SplitError::Cancelled);
}

#[test]
fn encrypted_files() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in ["encrypted_rc4.pdf", "encrypted_aes256.pdf"] {
        let path = fixtures.join(name);
        assert_eq!(
            Pdf::open(&path, None).err(),
            Some(SplitError::PasswordRequired),
            "{name}"
        );
        assert_eq!(
            Pdf::open(&path, Some("nope")).err(),
            Some(SplitError::WrongPassword),
            "{name}"
        );
        let pdf = Pdf::open(&path, Some("secret")).unwrap();
        assert_eq!(pdf.page_count(), 4);

        // Output is a normal, unencrypted PDF.
        let dir = tempfile::tempdir().unwrap();
        let groups = Mode::Ranges.groups(4, "2-3").unwrap();
        let files =
            write_groups(&pdf, &groups, dir.path(), "e", false, &no_cancel(), |_| {}).unwrap();
        let out = Document::load(&files[0]).unwrap();
        assert!(!out.trailer.has(b"Encrypt"), "{name}");
        assert_eq!(markers(&files[0]), vec![1, 2], "{name}");
    }
    // Only an owner password: opens without asking.
    let pdf = Pdf::open(&fixtures.join("encrypted_no_user_pw.pdf"), None).unwrap();
    assert_eq!(pdf.page_count(), 3);
}

#[test]
fn not_a_pdf() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fake.pdf");
    std::fs::write(&path, b"hello").unwrap();
    assert!(matches!(Pdf::open(&path, None), Err(SplitError::Pdf(_))));
}
