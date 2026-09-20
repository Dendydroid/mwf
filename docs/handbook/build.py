#!/usr/bin/env python3
"""Assemble the handbook fragments into one HTML file, then print it to PDF.

Why this exists rather than pandoc: nothing on this machine renders paged HTML.
What it does have is Chrome, and Chrome can print a page to PDF from the command
line. Paged.js turns a long HTML document into numbered pages with running heads
and a table of contents that knows its own page numbers - the three things
Chrome's own print CSS cannot do.

    python docs/handbook/build.py            assemble + render
    python docs/handbook/build.py --html     assemble only, skip Chrome
    python docs/handbook/build.py --only 01  assemble a subset, for a smoke test

Output lands in docs/ as call-assistant-handbook.{html,pdf}. The HTML is
standalone - CSS and Paged.js are inlined, fonts are system fonts - so it opens
in a browser with no network and no build step.
"""

from __future__ import annotations

import argparse
import re
import shutil
import sys
import time

import cdp
from pathlib import Path

HERE = Path(__file__).resolve().parent
DOCS = HERE.parent
OUT_HTML = DOCS / "call-assistant-handbook.html"
OUT_PDF = DOCS / "call-assistant-handbook.pdf"

CHROME_CANDIDATES = [
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
]

# Fragments are concatenated in filename order; the numeric prefix is the order.
FRAGMENT_GLOB = "[0-9][0-9]-*.html"


def find_chrome() -> str:
    for candidate in CHROME_CANDIDATES:
        if Path(candidate).exists():
            return candidate
    for name in ("chrome", "chromium", "msedge"):
        found = shutil.which(name)
        if found:
            return found
    sys.exit("No Chrome or Edge found. Pass --html to skip the PDF step.")


def fragments(only: list[str] | None) -> list[Path]:
    found = sorted(HERE.glob(FRAGMENT_GLOB))
    if not found:
        sys.exit(f"No fragments matching {FRAGMENT_GLOB} in {HERE}")
    if only:
        found = [f for f in found if any(f.name.startswith(p) for p in only)]
        if not found:
            sys.exit(f"--only {only} matched no fragments")
    return found


# The table of contents is generated from the fragments rather than maintained
# by hand, because a hand-kept one is wrong the moment a chapter is renumbered -
# and this book gets renumbered every time a part is rewritten. Two markup
# conventions are all it needs, and both are load-bearing:
#
#   part:     <h1 id="p3" data-part="Part III">The prompt layer</h1>
#   chapter:  <h2 id="c17"><span class="num">17</span>Rendering the schema</h2>
#
PART_RE = re.compile(
    r'<h1 id="(?P<id>[^"]+)" data-part="(?P<kicker>[^"]*)"[^>]*>(?P<title>.*?)</h1>',
    re.S,
)
CHAPTER_RE = re.compile(
    r'<h2 id="(?P<id>[^"]+)"[^>]*>\s*(?:<span class="num">(?P<num>[^<]*)</span>)?'
    r"\s*(?P<title>.*?)</h2>",
    re.S,
)
TAGS_RE = re.compile(r"<[^>]+>")


def strip_tags(text: str) -> str:
    return " ".join(TAGS_RE.sub("", text).split())


def build_toc(body: str) -> str:
    """Walk the assembled body in document order, emitting parts and chapters."""
    events = []
    for m in PART_RE.finditer(body):
        events.append((m.start(), "part", m.group("id"),
                       strip_tags(m.group("kicker")), strip_tags(m.group("title"))))
    for m in CHAPTER_RE.finditer(body):
        events.append((m.start(), "chapter", m.group("id"),
                       strip_tags(m.group("num") or ""), strip_tags(m.group("title"))))
    events.sort()

    lines = []
    for _, kind, anchor, label, title in events:
        if kind == "part":
            lines.append(f'<div class="toc-part">{label} &middot; {title}</div>')
        else:
            n = f'<span class="ch-n">{label}</span>' if label else ""
            lines.append(
                f'<a href="#{anchor}">{n}<span class="t">{title}</span>'
                f'<span class="dots"></span></a>'
            )
    return "\n".join(lines)


def assemble(only: list[str] | None) -> str:
    css = (HERE / "style.css").read_text(encoding="utf-8")
    paged = (HERE / "vendor" / "paged.polyfill.js").read_text(encoding="utf-8")

    body_parts = []
    for frag in fragments(only):
        text = frag.read_text(encoding="utf-8")
        body_parts.append(f"<!-- ===== {frag.name} ===== -->\n{text}")
    body = "\n\n".join(body_parts)

    if "<!--TOC-->" in body:
        toc = build_toc(body)
        body = body.replace("<!--TOC-->", toc)
        print(f"  contents: {toc.count('<a href')} chapters, "
              f"{toc.count('toc-part')} parts")

    # Paged.js signals completion by adding a class to <html>; the flag below is
    # what the render step polls for instead of guessing at a fixed delay.
    done_hook = (
        "<script>window.PagedConfig={auto:true,after:()=>{"
        "document.documentElement.setAttribute('data-pagedjs-done','1');}};</script>"
    )

    return (
        "<!DOCTYPE html>\n"
        '<html lang="en">\n<head>\n<meta charset="utf-8">\n'
        "<title>The Call Assistant Engineering Handbook</title>\n"
        '<meta name="viewport" content="width=device-width,initial-scale=1">\n'
        f"{done_hook}\n"
        f"<style>\n{css}\n</style>\n"
        f"<script>\n{paged}\n</script>\n"
        "</head>\n<body>\n"
        f"{body}\n"
        "</body>\n</html>\n"
    )


def render(chrome: str) -> None:
    """Load the document, wait for Paged.js to finish, then print.

    The waiting is the whole point. Chrome's --print-to-pdf does not wait for
    JavaScript: it printed a 4-page PDF of a 13-page book, with no error, until
    this was replaced with a DevTools session. See cdp.py for the details.
    """
    started = time.time()

    with cdp.Chrome(chrome, OUT_HTML.resolve().as_uri()) as browser:
        browser.wait_for(
            "document.documentElement.getAttribute('data-pagedjs-done') === '1'",
            timeout=600,
            what="Paged.js to finish paginating",
        )
        boxes = browser.eval("document.querySelectorAll('.pagedjs_page').length")
        pdf = browser.print_to_pdf(
            printBackground=True,
            preferCSSPageSize=True,
            marginTop=0, marginBottom=0, marginLeft=0, marginRight=0,
        )

    OUT_PDF.write_bytes(pdf)
    print(f"  paginated {boxes} pages, rendered in {time.time() - started:.1f}s")


def page_count(pdf: Path) -> int | None:
    """Page count without a PDF library: count /Type /Page objects."""
    try:
        raw = pdf.read_bytes()
    except OSError:
        return None
    counts = re.findall(rb"/Count\s+(\d+)", raw)
    if counts:
        return max(int(c) for c in counts)
    return len(re.findall(rb"/Type\s*/Page[^s]", raw)) or None


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--html", action="store_true", help="assemble only, skip Chrome")
    ap.add_argument("--only", nargs="*", help="filename prefixes to include")
    args = ap.parse_args()

    html = assemble(args.only)
    OUT_HTML.write_text(html, encoding="utf-8")
    print(f"  {OUT_HTML.relative_to(DOCS.parent)}  {len(html) / 1024:.0f} KB")

    if args.html:
        return

    render(find_chrome())
    pages = page_count(OUT_PDF)
    size = OUT_PDF.stat().st_size / 1024
    print(f"  {OUT_PDF.relative_to(DOCS.parent)}  {size:.0f} KB"
          + (f"  {pages} pages" if pages else ""))


if __name__ == "__main__":
    main()
