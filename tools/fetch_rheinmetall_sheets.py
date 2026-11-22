#!/usr/bin/env python3
"""Download Rheinmetall's public product datasheets (brochure PDFs) in one go.

Finds the product pages from the site's sitemap and from every product path
mentioned anywhere in each page (not only in plain links: much of the site's
navigation is in scripts), collects every brochure PDF they mention, and
saves each one. Standard library only.

    python3 fetch_rheinmetall_sheets.py                      # weapons and ammunition, into ./rheinmetall_sheets
    python3 fetch_rheinmetall_sheets.py --all                # every product family (vehicles, air defence, ...)
    python3 fetch_rheinmetall_sheets.py --out DIR --debug    # also save each page's HTML, to see why nothing was found

It stays on rheinmetall.com, waits between requests, and skips files it
already has, so it is safe to re-run.
"""
import argparse
import gzip
import html
import os
import re
import sys
import time
import urllib.parse
import urllib.request

SITE = "https://www.rheinmetall.com"
UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36"
# Any product path or PDF mentioned in the page, in a link, a script or JSON.
PAGE = re.compile(r"(/en/products/[A-Za-z0-9_\-/]+)")
PDF = re.compile(r"""((?:https?://www\.rheinmetall\.com)?/[^"'<>\s()]*?\.pdf)""", re.I)
LOC = re.compile(r"<loc>\s*([^<\s]+)\s*</loc>", re.I)


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "*/*"})
    with urllib.request.urlopen(req, timeout=60) as r:
        data = r.read()
    if data[:2] == b"\x1f\x8b":
        data = gzip.decompress(data)
    return data


def text(data):
    t = data.decode("utf-8", "replace")
    # Paths inside scripts and JSON are often escaped.
    t = t.replace("\\/", "/").replace("\\u002F", "/").replace("\\u002f", "/")
    return html.unescape(t)


def sitemap_pages(prefix, delay):
    """Product pages listed in the site's sitemaps."""
    found, todo, seen = set(), [SITE + "/sitemap.xml"], set()
    try:
        robots = text(get(SITE + "/robots.txt"))
        todo += re.findall(r"(?im)^\s*sitemap:\s*(\S+)", robots)
    except Exception:  # noqa: BLE001
        pass
    while todo:
        u = todo.pop()
        if u in seen:
            continue
        seen.add(u)
        try:
            body = text(get(u))
        except Exception as e:  # noqa: BLE001
            print(f"  no sitemap at {u}: {e}", file=sys.stderr)
            continue
        for loc in LOC.findall(body):
            if loc.endswith(".xml") or loc.endswith(".xml.gz"):
                todo.append(loc)
            elif urllib.parse.urlparse(loc).path.startswith(prefix):
                found.add(loc.split("?")[0].rstrip("/"))
            elif loc.lower().endswith(".pdf"):
                found.add(loc)
        time.sleep(delay)
    return found


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="rheinmetall_sheets")
    ap.add_argument("--all", action="store_true", help="every product family, not just weapons and ammunition")
    ap.add_argument("--max-pages", type=int, default=600)
    ap.add_argument("--delay", type=float, default=1.0, help="seconds between requests")
    ap.add_argument("--debug", action="store_true", help="save each page's HTML into OUT/pages")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    if a.debug:
        os.makedirs(os.path.join(a.out, "pages"), exist_ok=True)

    prefix = "/en/products" if a.all else "/en/products/weapons-and-ammunition"
    pdfs = {}
    from_map = sitemap_pages(prefix, a.delay)
    for u in [u for u in from_map if u.lower().endswith(".pdf")]:
        pdfs.setdefault(u, "sitemap")
    queue = [SITE + prefix] + sorted(u for u in from_map if not u.lower().endswith(".pdf"))
    print(f"{len(queue)} pages to start from ({len(from_map)} from the sitemap)")

    seen = set()
    while queue and len(seen) < a.max_pages:
        url = queue.pop(0).rstrip("/")
        if url in seen:
            continue
        seen.add(url)
        try:
            page = text(get(url))
        except Exception as e:  # noqa: BLE001
            print(f"  skip page {url}: {e}", file=sys.stderr)
            continue
        if a.debug:
            name = re.sub(r"[^A-Za-z0-9]+", "_", urllib.parse.urlparse(url).path).strip("_") or "index"
            with open(os.path.join(a.out, "pages", name + ".html"), "w", encoding="utf-8") as f:
                f.write(page)
        new = 0
        for m in PDF.findall(page):
            link = urllib.parse.urljoin(SITE, m)
            if link not in pdfs:
                pdfs[link] = url
                new += 1
        for p in PAGE.findall(page):
            link = (SITE + p).rstrip("/")
            if p.startswith(prefix) and link not in seen:
                queue.append(link)
        print(f"page {len(seen):3d}  +{new:2d} pdf  {url}")
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
            data = get(urllib.parse.quote(link, safe=":/%?=&"))
            if not data.startswith(b"%PDF"):
                print(f"  not a PDF, skipped: {link}", file=sys.stderr)
                continue
            with open(dest, "wb") as f:
                f.write(data)
            print(f"  saved {name} ({len(data) // 1024} KB)")
        except Exception as e:  # noqa: BLE001
            print(f"  failed {name}: {e}", file=sys.stderr)
        time.sleep(a.delay)
    with open(os.path.join(a.out, "SOURCES.tsv"), "w", encoding="utf-8") as f:
        f.write("file\turl\tfound_on_page\n" + "\n".join(index) + "\n")
    print(f"done; files and SOURCES.tsv in {os.path.abspath(a.out)}")
    if not pdfs:
        print("No PDFs found. Re-run with --debug and send me one of the saved pages in OUT/pages.")


if __name__ == "__main__":
    main()
