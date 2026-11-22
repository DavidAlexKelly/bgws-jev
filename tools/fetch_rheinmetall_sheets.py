#!/usr/bin/env python3
"""Download Rheinmetall's public product datasheets (brochure PDFs) in one go.

Walks the weapons-and-ammunition product pages on rheinmetall.com, collects
every link to a brochure PDF, and saves each one. Standard library only.

    python3 tools/fetch_rheinmetall_sheets.py                # into docs/papers/rheinmetall
    python3 tools/fetch_rheinmetall_sheets.py --out DIR --all  # every product family, not just weapons and ammunition

It only follows links on rheinmetall.com under the start pages, waits between
requests, and skips files it already has, so it is safe to re-run.
"""
import argparse
import html
import os
import re
import sys
import time
import urllib.parse
import urllib.request

SITE = "https://www.rheinmetall.com"
STARTS = {
    "weapons": ["/en/products/weapons-and-ammunition"],
    "all": ["/en/products"],
}
UA = "Mozilla/5.0 (datasheet fetcher for research)"
HREF = re.compile(r"""href\s*=\s*["']([^"'#]+)["']""", re.I)


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=os.path.join(os.path.dirname(__file__), "..", "docs", "papers", "rheinmetall"))
    ap.add_argument("--all", action="store_true", help="crawl every product family, not just weapons and ammunition")
    ap.add_argument("--max-pages", type=int, default=400)
    ap.add_argument("--delay", type=float, default=1.0, help="seconds between requests")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)

    prefixes = STARTS["all" if a.all else "weapons"]
    queue = [SITE + p for p in prefixes]
    seen, pdfs = set(), {}
    while queue and len(seen) < a.max_pages:
        url = queue.pop(0)
        if url in seen:
            continue
        seen.add(url)
        try:
            page = get(url).decode("utf-8", "replace")
        except Exception as e:  # noqa: BLE001
            print(f"  skip page {url}: {e}", file=sys.stderr)
            continue
        print(f"page {len(seen):3d}  {url}")
        for href in HREF.findall(page):
            link = urllib.parse.urljoin(url, html.unescape(href))
            path = urllib.parse.urlparse(link).path
            if not link.startswith(SITE):
                continue
            if path.lower().endswith(".pdf"):
                pdfs.setdefault(link, url)
            elif any(path.startswith(p) for p in prefixes) and link not in seen:
                queue.append(link.split("?")[0])
        time.sleep(a.delay)

    print(f"\n{len(pdfs)} PDFs found on {len(seen)} pages")
    index = []
    for link, page in sorted(pdfs.items()):
        name = urllib.parse.unquote(os.path.basename(urllib.parse.urlparse(link).path))
        dest = os.path.join(a.out, name)
        index.append(f"{name}\t{link}\t{page}")
        if os.path.exists(dest):
            continue
        try:
            data = get(link)
            with open(dest, "wb") as f:
                f.write(data)
            print(f"  saved {name} ({len(data) // 1024} KB)")
        except Exception as e:  # noqa: BLE001
            print(f"  failed {name}: {e}", file=sys.stderr)
        time.sleep(a.delay)
    # Where each file came from, for citing it.
    with open(os.path.join(a.out, "SOURCES.tsv"), "w", encoding="utf-8") as f:
        f.write("file\turl\tfound_on_page\n" + "\n".join(index) + "\n")
    print(f"done; list of sources in {os.path.join(a.out, 'SOURCES.tsv')}")


if __name__ == "__main__":
    main()
