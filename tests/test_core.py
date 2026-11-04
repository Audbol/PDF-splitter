from pathlib import Path

import pytest
from pypdf import PdfReader, PdfWriter

from pdf_splitter.core import (
    SplitError,
    group_extract,
    groups_every_n,
    groups_every_page,
    groups_from_ranges,
    groups_into_parts,
    open_pdf,
    parse_ranges,
    write_groups,
)


@pytest.fixture
def sample_pdf(tmp_path: Path) -> Path:
    writer = PdfWriter()
    for i in range(10):
        writer.add_blank_page(width=100 + i, height=100)  # width encodes page no.
    path = tmp_path / "sample.pdf"
    with open(path, "wb") as fh:
        writer.write(fh)
    return path


def widths(path: Path) -> list[int]:
    return [int(p.mediabox.width) - 100 for p in PdfReader(str(path)).pages]


def test_parse_ranges_variants():
    assert parse_ranges("1-3, 5", 10) == [[0, 1, 2], [4]]
    assert parse_ranges("8-", 10) == [[7, 8, 9]]
    assert parse_ranges("-2", 10) == [[0, 1]]
    assert parse_ranges("3-1; last", 10) == [[2, 1, 0], [9]]
    assert parse_ranges("9-end", 10) == [[8, 9]]
    assert parse_ranges("-", 3) == [[0, 1, 2]]


@pytest.mark.parametrize("spec", ["", "abc", "0", "11", "1-20", "1--2"])
def test_parse_ranges_invalid(spec):
    with pytest.raises(SplitError):
        parse_ranges(spec, 10)


def test_group_modes():
    assert len(groups_every_page(4)) == 4
    assert [g.pages for g in groups_every_n(5, 2)] == [(0, 1), (2, 3), (4,)]
    assert [len(g.pages) for g in groups_into_parts(10, 3)] == [4, 3, 3]
    assert len(groups_into_parts(2, 5)) == 2
    assert [g.label for g in groups_from_ranges("1-3,5", 10)] == ["1-3", "5"]
    assert group_extract("1,3-4", 10)[0].pages == (0, 2, 3)


def test_write_every_page(sample_pdf, tmp_path):
    reader = open_pdf(sample_pdf)
    calls = []
    files = write_groups(reader, groups_every_page(10), tmp_path / "out", "doc",
                         progress=lambda d, t: calls.append((d, t)))
    assert [f.name for f in files][:2] == ["doc_01.pdf", "doc_02.pdf"]
    assert widths(files[4]) == [4]
    assert calls[-1] == (10, 10)


def test_write_ranges_and_extract(sample_pdf, tmp_path):
    reader = open_pdf(sample_pdf)
    files = write_groups(reader, groups_from_ranges("2-4, 10", 10), tmp_path, "x")
    assert [f.name for f in files] == ["x_02-04.pdf", "x_10.pdf"]
    assert widths(files[0]) == [1, 2, 3]
    (out,) = write_groups(reader, group_extract("5, 1-2", 10), tmp_path, "x")
    assert widths(out) == [4, 0, 1]


def test_no_overwrite_by_default(sample_pdf, tmp_path):
    reader = open_pdf(sample_pdf)
    a = write_groups(reader, groups_every_n(10, 10), tmp_path, "d")
    b = write_groups(reader, groups_every_n(10, 10), tmp_path, "d")
    assert a[0] != b[0] and b[0].name == "d_01-10 (2).pdf"
    c = write_groups(reader, groups_every_n(10, 10), tmp_path, "d", overwrite=True)
    assert c[0] == a[0]


def test_encrypted(tmp_path, sample_pdf):
    writer = PdfWriter(clone_from=str(sample_pdf))
    writer.encrypt("secret")
    path = tmp_path / "enc.pdf"
    with open(path, "wb") as fh:
        writer.write(fh)
    with pytest.raises(SplitError, match="password"):
        open_pdf(path)
    assert len(open_pdf(path, "secret").pages) == 10
