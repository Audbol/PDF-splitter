"""Core PDF splitting logic, independent of the GUI."""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterable, Sequence

from pypdf import PdfReader, PdfWriter

ProgressCallback = Callable[[int, int], None]


class SplitError(ValueError):
    """Raised for invalid user input (bad ranges, unreadable files...)."""


@dataclass(frozen=True)
class PageGroup:
    """A group of 0-based page indices that will become one output file."""

    pages: tuple[int, ...]
    label: str  # human readable, 1-based, e.g. "1-3" or "5"


def _label(pages: Sequence[int]) -> str:
    first, last = pages[0] + 1, pages[-1] + 1
    contiguous = list(pages) == list(range(pages[0], pages[-1] + 1))
    if first == last:
        return str(first)
    return f"{first}-{last}" if contiguous else f"{first}-{last}_sel"


_TOKEN = re.compile(r"^\s*(\d*)\s*(?:(-)\s*(\d*))?\s*$")


def parse_ranges(spec: str, page_count: int) -> list[list[int]]:
    """Parse a range spec like ``"1-3, 5, 8-"`` into lists of 0-based indices.

    * ``N``     -> single page
    * ``A-B``   -> pages A..B inclusive (reverse order allowed: ``5-3``)
    * ``A-``    -> page A to the last page
    * ``-B``    -> first page to page B
    * ``last`` / ``end`` may be used as a page number.

    Ranges are separated by commas or semicolons. Each range is returned as
    its own list, in the order given.
    """
    if page_count <= 0:
        raise SplitError("The document has no pages.")
    spec = re.sub(r"\b(last|end)\b", str(page_count), spec, flags=re.IGNORECASE)
    parts = [p for p in re.split(r"[,;]", spec) if p.strip()]
    if not parts:
        raise SplitError("Please enter at least one page or range.")

    result: list[list[int]] = []
    for part in parts:
        m = _TOKEN.match(part)
        if not m or (not m.group(1) and not m.group(2)):
            raise SplitError(f"Invalid range: '{part.strip()}'")
        start_s, dash, end_s = m.groups()
        if dash:
            start = int(start_s) if start_s else 1
            end = int(end_s) if end_s else page_count
        else:
            start = end = int(start_s)
        for n in (start, end):
            if n < 1 or n > page_count:
                raise SplitError(
                    f"Page {n} in '{part.strip()}' is out of range (1-{page_count})."
                )
        step = 1 if end >= start else -1
        result.append(list(range(start - 1, end - 1 + step, step)))
    return result


def groups_every_page(page_count: int) -> list[PageGroup]:
    return [PageGroup((i,), str(i + 1)) for i in range(page_count)]


def groups_every_n(page_count: int, n: int) -> list[PageGroup]:
    if n < 1:
        raise SplitError("Pages per file must be at least 1.")
    groups = []
    for start in range(0, page_count, n):
        pages = tuple(range(start, min(start + n, page_count)))
        groups.append(PageGroup(pages, _label(pages)))
    return groups


def groups_into_parts(page_count: int, parts: int) -> list[PageGroup]:
    """Split into ``parts`` files of (nearly) equal size."""
    if parts < 1:
        raise SplitError("Number of files must be at least 1.")
    parts = min(parts, page_count)
    base, extra = divmod(page_count, parts)
    groups, start = [], 0
    for i in range(parts):
        size = base + (1 if i < extra else 0)
        pages = tuple(range(start, start + size))
        groups.append(PageGroup(pages, _label(pages)))
        start += size
    return groups


def groups_from_ranges(spec: str, page_count: int) -> list[PageGroup]:
    return [PageGroup(tuple(p), _label(p)) for p in parse_ranges(spec, page_count)]


def group_extract(spec: str, page_count: int) -> list[PageGroup]:
    """All pages from the spec merged into a single output file."""
    pages: list[int] = []
    for rng in parse_ranges(spec, page_count):
        pages.extend(rng)
    return [PageGroup(tuple(pages), "extract")]


def safe_stem(name: str) -> str:
    """Make a string safe to use as a Windows file name stem."""
    name = re.sub(r'[<>:"/\\|?*\x00-\x1f]', "_", name).strip(" .")
    return name or "document"


def open_pdf(path: str | Path, password: str | None = None) -> PdfReader:
    try:
        reader = PdfReader(str(path))
    except Exception as exc:  # pypdf raises a variety of errors
        raise SplitError(f"Could not open PDF: {exc}") from exc
    if reader.is_encrypted:
        try:
            ok = reader.decrypt(password or "")
        except Exception as exc:
            raise SplitError(f"Could not decrypt PDF: {exc}") from exc
        if not ok:
            raise SplitError("This PDF is password protected.")
    return reader


def write_groups(
    reader: PdfReader,
    groups: Iterable[PageGroup],
    out_dir: str | Path,
    prefix: str,
    progress: ProgressCallback | None = None,
    overwrite: bool = False,
) -> list[Path]:
    """Write each group to ``out_dir/<prefix>_<label>.pdf``. Returns the paths."""
    groups = list(groups)
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    prefix = safe_stem(prefix)
    total_pages = sum(len(g.pages) for g in groups) or 1
    done = 0
    written: list[Path] = []
    page_count = len(reader.pages)

    for group in groups:
        writer = PdfWriter()
        for idx in group.pages:
            writer.add_page(reader.pages[idx])
            done += 1
            if progress:
                progress(done, total_pages)
        target = _unique(out_dir / output_name(prefix, group, page_count), overwrite)
        with open(target, "wb") as fh:
            writer.write(fh)
        written.append(target)
    return written


def output_name(prefix: str, group: PageGroup, page_count: int) -> str:
    """File name for a group, with page numbers zero-padded so they sort well."""
    width = len(str(page_count))
    label = "-".join(
        part.zfill(width) if part.isdigit() else part for part in group.label.split("-")
    )
    return f"{safe_stem(prefix)}_{label}.pdf"


def _unique(path: Path, overwrite: bool) -> Path:
    if overwrite or not path.exists():
        return path
    i = 2
    while True:
        candidate = path.with_name(f"{path.stem} ({i}){path.suffix}")
        if not candidate.exists():
            return candidate
        i += 1
