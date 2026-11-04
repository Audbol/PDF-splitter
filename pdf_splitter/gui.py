"""CustomTkinter GUI for PDF Splitter."""

from __future__ import annotations

import os
import queue
import subprocess
import sys
import threading
from pathlib import Path
from tkinter import filedialog, messagebox

import customtkinter as ctk

from . import __version__
from .core import (
    SplitError,
    group_extract,
    groups_every_n,
    groups_every_page,
    groups_from_ranges,
    groups_into_parts,
    open_pdf,
    output_name,
    write_groups,
)

try:  # Optional: drag & drop support
    from tkinterdnd2 import DND_FILES, TkinterDnD

    _DND = True
except Exception:  # pragma: no cover - depends on platform
    _DND = False

try:  # Optional: first page thumbnail
    import pypdfium2 as pdfium

    _PREVIEW = True
except Exception:  # pragma: no cover
    _PREVIEW = False

ACCENT = ("#2563eb", "#3b82f6")
MUTED = ("gray40", "gray65")
CARD = ("gray95", "gray17")

MODES = {
    "Every page": "Each page becomes its own PDF file.",
    "Every N pages": "Cut the document into chunks of N pages.",
    "Equal parts": "Divide the document into N files of equal size.",
    "Custom ranges": "One file per range, e.g.  1-3, 5, 8-last",
    "Extract pages": "Pick pages and merge them into a single PDF, e.g.  1, 4-6, 9",
}


def resource_path(name: str) -> Path:
    base = Path(getattr(sys, "_MEIPASS", Path(__file__).resolve().parent.parent))
    return base / "assets" / name


def open_folder(path: Path) -> None:
    try:
        if sys.platform.startswith("win"):
            os.startfile(path)  # type: ignore[attr-defined]
        elif sys.platform == "darwin":
            subprocess.Popen(["open", str(path)])
        else:
            subprocess.Popen(["xdg-open", str(path)])
    except Exception:
        pass


_Base = (type("_Base", (ctk.CTk, TkinterDnD.DnDWrapper), {}) if _DND else ctk.CTk)


class App(_Base):  # type: ignore[misc, valid-type]
    def __init__(self, initial_file: str | None = None) -> None:
        super().__init__()
        if _DND:
            try:
                self.TkdndVersion = TkinterDnD._require(self)
            except Exception:
                pass

        self.title("PDF Splitter")
        self.geometry("820x640")
        self.minsize(720, 580)
        icon = resource_path("icon.ico")
        if sys.platform.startswith("win") and icon.exists():
            try:
                self.iconbitmap(str(icon))
            except Exception:
                pass

        self.reader = None
        self.pdf_path: Path | None = None
        self.last_output: Path | None = None
        self._queue: queue.Queue = queue.Queue()
        self._busy = False
        self._thumb = None

        self.grid_columnconfigure(0, weight=1)
        self.grid_rowconfigure(2, weight=1)
        self._build_header()
        self._build_file_card()
        self._build_options()
        self._build_footer()
        self._set_mode("Every page")
        self._refresh_state()

        if initial_file:
            self.after(100, lambda: self.load_file(initial_file))

    # ------------------------------------------------------------------ UI
    def _build_header(self) -> None:
        header = ctk.CTkFrame(self, fg_color="transparent")
        header.grid(row=0, column=0, sticky="ew", padx=24, pady=(20, 8))
        header.grid_columnconfigure(0, weight=1)
        ctk.CTkLabel(
            header, text="PDF Splitter", font=ctk.CTkFont(size=26, weight="bold")
        ).grid(row=0, column=0, sticky="w")
        ctk.CTkLabel(
            header,
            text="Split PDFs into pages, chunks or custom ranges",
            text_color=MUTED,
        ).grid(row=1, column=0, sticky="w")
        self.theme_switch = ctk.CTkSegmentedButton(
            header, values=["System", "Light", "Dark"], command=ctk.set_appearance_mode
        )
        self.theme_switch.set("System")
        self.theme_switch.grid(row=0, column=1, rowspan=2, sticky="e")

    def _build_file_card(self) -> None:
        card = ctk.CTkFrame(self, corner_radius=14, fg_color=CARD)
        card.grid(row=1, column=0, sticky="ew", padx=24, pady=8)
        card.grid_columnconfigure(1, weight=1)
        self.drop_target = card

        self.thumb_label = ctk.CTkLabel(
            card, text="PDF", width=84, height=110, corner_radius=8,
            fg_color=("gray85", "gray25"), font=ctk.CTkFont(size=18, weight="bold"),
            text_color=MUTED,
        )
        self.thumb_label.grid(row=0, column=0, rowspan=3, padx=16, pady=16)

        self.file_title = ctk.CTkLabel(
            card, text="No file selected", anchor="w",
            font=ctk.CTkFont(size=16, weight="bold"),
        )
        self.file_title.grid(row=0, column=1, sticky="ew", pady=(18, 0))
        hint = "Drop a PDF here or click Browse" if _DND else "Click Browse to choose a PDF"
        self.file_info = ctk.CTkLabel(card, text=hint, anchor="w", text_color=MUTED)
        self.file_info.grid(row=1, column=1, sticky="ew")

        ctk.CTkButton(card, text="Browse…", width=110, command=self.browse_file).grid(
            row=0, column=2, rowspan=3, padx=16
        )

        if _DND:
            for widget in (card, self.thumb_label, self.file_title, self.file_info):
                try:
                    widget.drop_target_register(DND_FILES)
                    widget.dnd_bind("<<Drop>>", self._on_drop)
                except Exception:
                    pass

    def _build_options(self) -> None:
        card = ctk.CTkFrame(self, corner_radius=14, fg_color=CARD)
        card.grid(row=2, column=0, sticky="nsew", padx=24, pady=8)
        card.grid_columnconfigure(1, weight=1)

        ctk.CTkLabel(card, text="Split mode", font=ctk.CTkFont(weight="bold")).grid(
            row=0, column=0, sticky="w", padx=16, pady=(16, 6)
        )
        self.mode_btn = ctk.CTkSegmentedButton(
            card, values=list(MODES), command=self._set_mode
        )
        self.mode_btn.grid(row=0, column=1, columnspan=2, sticky="ew", padx=16, pady=(16, 6))

        self.mode_desc = ctk.CTkLabel(card, text="", anchor="w", text_color=MUTED)
        self.mode_desc.grid(row=1, column=1, columnspan=2, sticky="ew", padx=16)

        # Mode specific input
        self.param_label = ctk.CTkLabel(card, text="", font=ctk.CTkFont(weight="bold"))
        self.param_label.grid(row=2, column=0, sticky="w", padx=16, pady=10)
        self.param_entry = ctk.CTkEntry(card)
        self.param_entry.grid(row=2, column=1, columnspan=2, sticky="ew", padx=16, pady=10)
        self.param_entry.bind("<KeyRelease>", lambda _e: self._update_preview())

        ctk.CTkLabel(card, text="Output folder", font=ctk.CTkFont(weight="bold")).grid(
            row=3, column=0, sticky="w", padx=16, pady=6
        )
        self.out_entry = ctk.CTkEntry(card, placeholder_text="Default: a \"<name>_split\" folder next to the PDF")
        self.out_entry.grid(row=3, column=1, sticky="ew", padx=(16, 8), pady=6)
        ctk.CTkButton(card, text="Choose…", width=90, command=self.browse_output).grid(
            row=3, column=2, padx=(0, 16), pady=6
        )

        ctk.CTkLabel(card, text="File name prefix", font=ctk.CTkFont(weight="bold")).grid(
            row=4, column=0, sticky="w", padx=16, pady=6
        )
        self.prefix_entry = ctk.CTkEntry(card, placeholder_text="Defaults to the PDF name")
        self.prefix_entry.grid(row=4, column=1, columnspan=2, sticky="ew", padx=16, pady=6)
        self.prefix_entry.bind("<KeyRelease>", lambda _e: self._update_preview())

        self.overwrite_var = ctk.BooleanVar(value=False)
        ctk.CTkCheckBox(
            card, text="Overwrite existing files", variable=self.overwrite_var
        ).grid(row=5, column=1, sticky="w", padx=16, pady=6)

        self.preview = ctk.CTkLabel(
            card, text="", anchor="w", justify="left", text_color=ACCENT, wraplength=520
        )
        self.preview.grid(row=6, column=0, columnspan=3, sticky="ew", padx=16, pady=(6, 16))

    def _build_footer(self) -> None:
        footer = ctk.CTkFrame(self, fg_color="transparent")
        footer.grid(row=3, column=0, sticky="ew", padx=24, pady=(8, 20))
        footer.grid_columnconfigure(0, weight=1)

        self.progress = ctk.CTkProgressBar(footer)
        self.progress.set(0)
        self.progress.grid(row=0, column=0, sticky="ew", padx=(0, 16))
        self.status = ctk.CTkLabel(
            footer, text=f"v{__version__}", anchor="w", text_color=MUTED
        )
        self.status.grid(row=1, column=0, sticky="ew")

        self.open_btn = ctk.CTkButton(
            footer, text="Open folder", width=120, fg_color="transparent",
            border_width=2, text_color=("gray10", "gray90"),
            command=lambda: self.last_output and open_folder(self.last_output),
        )
        self.open_btn.grid(row=0, column=1, rowspan=2, padx=(0, 10))
        self.split_btn = ctk.CTkButton(
            footer, text="Split PDF", width=150, height=42,
            font=ctk.CTkFont(size=15, weight="bold"), command=self.start_split,
        )
        self.split_btn.grid(row=0, column=2, rowspan=2)

    # ------------------------------------------------------------ helpers
    def _set_mode(self, mode: str) -> None:
        self.mode_btn.set(mode)
        self.mode_desc.configure(text=MODES[mode])
        self.param_entry.delete(0, "end")
        if mode == "Every page":
            self.param_label.configure(text="")
            self.param_entry.grid_remove()
        else:
            self.param_entry.grid()
            if mode in ("Every N pages", "Equal parts"):
                label = "Pages per file" if mode == "Every N pages" else "Number of files"
                self.param_label.configure(text=label)
                self.param_entry.configure(placeholder_text="e.g. 2")
                self.param_entry.insert(0, "2")
            else:
                self.param_label.configure(text="Pages")
                self.param_entry.configure(placeholder_text="e.g. 1-3, 5, 8-last")
        self._update_preview()

    def _groups(self):
        count = len(self.reader.pages)
        mode = self.mode_btn.get()
        value = self.param_entry.get().strip()
        if mode == "Every page":
            return groups_every_page(count)
        if mode in ("Every N pages", "Equal parts"):
            try:
                n = int(value)
            except ValueError:
                raise SplitError("Please enter a whole number.") from None
            return groups_every_n(count, n) if mode == "Every N pages" else groups_into_parts(count, n)
        if mode == "Custom ranges":
            return groups_from_ranges(value, count)
        return group_extract(value, count)

    def _prefix(self) -> str:
        return self.prefix_entry.get().strip() or (self.pdf_path.stem if self.pdf_path else "document")

    def _output_dir(self) -> Path:
        value = self.out_entry.get().strip()
        if value:
            return Path(value)
        return self.pdf_path.parent / f"{self.pdf_path.stem}_split"

    def _update_preview(self) -> None:
        if not self.reader:
            self.preview.configure(text="")
            return
        try:
            groups = self._groups()
        except SplitError as exc:
            self.preview.configure(text=f"⚠  {exc}", text_color=("#b91c1c", "#f87171"))
            return
        n = len(groups)
        count = len(self.reader.pages)
        names = ", ".join(output_name(self._prefix(), g, count) for g in groups[:3])
        more = f", … (+{n - 3} more)" if n > 3 else ""
        self.preview.configure(
            text=f"→  {n} file{'s' if n != 1 else ''} will be created:  {names}{more}",
            text_color=ACCENT,
        )

    def _refresh_state(self) -> None:
        ready = self.reader is not None and not self._busy
        self.split_btn.configure(state="normal" if ready else "disabled")
        self.open_btn.configure(state="normal" if self.last_output and not self._busy else "disabled")

    def _set_thumbnail(self, path: Path) -> None:
        self._thumb = None
        if _PREVIEW:
            try:
                doc = pdfium.PdfDocument(str(path))
                page = doc[0]
                w, h = page.get_size()
                scale = 220 / max(h, 1)
                img = page.render(scale=scale).to_pil()
                doc.close()
                img.thumbnail((84, 110))
                self._thumb = ctk.CTkImage(light_image=img, dark_image=img, size=img.size)
            except Exception:
                self._thumb = None
        if self._thumb:
            self.thumb_label.configure(image=self._thumb, text="")
        else:
            self.thumb_label.configure(image=None, text="PDF")

    # ------------------------------------------------------------ actions
    def _on_drop(self, event) -> None:
        files = self.tk.splitlist(event.data)
        pdfs = [f for f in files if f.lower().endswith(".pdf")]
        if pdfs:
            self.load_file(pdfs[0])
        else:
            messagebox.showwarning("PDF Splitter", "Please drop a PDF file.")

    def browse_file(self) -> None:
        path = filedialog.askopenfilename(
            title="Choose a PDF", filetypes=[("PDF files", "*.pdf"), ("All files", "*.*")]
        )
        if path:
            self.load_file(path)

    def browse_output(self) -> None:
        path = filedialog.askdirectory(title="Choose output folder")
        if path:
            self.out_entry.delete(0, "end")
            self.out_entry.insert(0, path)

    def load_file(self, path: str) -> None:
        if self._busy:
            return
        p = Path(path).expanduser().resolve()
        try:
            reader = open_pdf(p)
        except SplitError as exc:
            if "password" not in str(exc).lower():
                messagebox.showerror("PDF Splitter", str(exc))
                return
            pw = ctk.CTkInputDialog(text="This PDF is password protected.\nPassword:", title="Password").get_input()
            if pw is None:
                return
            try:
                reader = open_pdf(p, pw)
            except SplitError as exc2:
                messagebox.showerror("PDF Splitter", str(exc2))
                return

        self.reader, self.pdf_path, self.last_output = reader, p, None
        pages = len(reader.pages)
        size_kb = p.stat().st_size / 1024
        size = f"{size_kb / 1024:.1f} MB" if size_kb > 1024 else f"{size_kb:.0f} KB"
        self.file_title.configure(text=p.name)
        folder = str(p.parent)
        if len(folder) > 48:
            folder = "…" + folder[-47:]
        self.file_info.configure(text=f"{pages} page{'s' if pages != 1 else ''}  •  {size}  •  {folder}")
        self._set_thumbnail(p)
        self.progress.set(0)
        self.status.configure(text="Ready")
        self._update_preview()
        self._refresh_state()

    def start_split(self) -> None:
        if not self.reader or self._busy:
            return
        try:
            groups = self._groups()
        except SplitError as exc:
            messagebox.showerror("PDF Splitter", str(exc))
            return
        out_dir, prefix, overwrite = self._output_dir(), self._prefix(), self.overwrite_var.get()

        self._busy = True
        self._refresh_state()
        self.progress.set(0)
        self.status.configure(text="Splitting…")

        def worker():
            try:
                files = write_groups(
                    self.reader, groups, out_dir, prefix,
                    progress=lambda d, t: self._queue.put(("progress", d / t)),
                    overwrite=overwrite,
                )
                self._queue.put(("done", (out_dir, files)))
            except Exception as exc:  # report any failure to the UI
                self._queue.put(("error", exc))

        threading.Thread(target=worker, daemon=True).start()
        self.after(50, self._poll)

    def _poll(self) -> None:
        try:
            while True:
                kind, payload = self._queue.get_nowait()
                if kind == "progress":
                    self.progress.set(payload)
                elif kind == "done":
                    out_dir, files = payload
                    self._busy = False
                    self.last_output = out_dir
                    self.progress.set(1)
                    self.status.configure(text=f"✓  Created {len(files)} file(s) in \"{out_dir.name}\"")
                    self._refresh_state()
                    return
                elif kind == "error":
                    self._busy = False
                    self.progress.set(0)
                    self.status.configure(text="Failed")
                    self._refresh_state()
                    messagebox.showerror("PDF Splitter", f"Splitting failed:\n{payload}")
                    return
        except queue.Empty:
            pass
        self.after(50, self._poll)


def main() -> None:
    ctk.set_appearance_mode("System")
    ctk.set_default_color_theme("blue")
    initial = sys.argv[1] if len(sys.argv) > 1 else None
    App(initial).mainloop()


if __name__ == "__main__":
    main()
