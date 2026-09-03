#!/usr/bin/env python3

import csv
import json
import sys


def main(argv):
    if len(argv) != 3:
        print("usage: slice-text-finder.py QUERY TESSERACT_TSV", file=sys.stderr)
        return 2
    query = " ".join(argv[1].casefold().split())
    if not query:
        print("query must not be empty", file=sys.stderr)
        return 2

    matches = find_matches(query, read_lines(argv[2]))
    if not matches:
        print("null")
        return 1
    for match in matches:
        print(json.dumps(match, ensure_ascii=False, separators=(",", ":")))
    return 0


def read_lines(path):
    lines = {}
    with open(path, newline="", encoding="utf-8") as handle:
        for row in csv.DictReader(handle, delimiter="\t"):
            text = (row.get("text") or "").strip()
            if not text:
                continue
            key = tuple(
                int(row.get(field) or 0)
                for field in ("page_num", "block_num", "par_num", "line_num")
            )
            lines.setdefault(key, []).append(row)
    return [
        sorted(rows, key=lambda row: int(row.get("word_num") or 0))
        for _, rows in sorted(lines.items())
    ]


def find_matches(query, lines):
    matches = []
    for words in lines:
        normalized_words = [
            " ".join((row.get("text") or "").strip().casefold().split())
            for row in words
        ]
        searchable = " ".join(normalized_words)
        word_spans = []
        offset = 0
        for word in normalized_words:
            word_spans.append((offset, offset + len(word)))
            offset += len(word) + 1

        cursor = 0
        while (found := searchable.find(query, cursor)) >= 0:
            found_end = found + len(query)
            selected = [
                row
                for row, (start, end) in zip(words, word_spans)
                if end > found and start < found_end
            ]
            if selected:
                matches.append(match_for_rows(selected))
            cursor = found_end
    return matches


def match_for_rows(rows):
    left = min(int(row["left"]) for row in rows)
    top = min(int(row["top"]) for row in rows)
    right = max(int(row["left"]) + int(row["width"]) for row in rows)
    bottom = max(int(row["top"]) + int(row["height"]) for row in rows)
    return {
        "text": " ".join((row.get("text") or "").strip() for row in rows),
        "left": left,
        "top": top,
        "width": right - left,
        "height": bottom - top,
        "center_x": (left + right) // 2,
        "center_y": (top + bottom) // 2,
    }


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv))
    except (OSError, TypeError, ValueError) as error:
        print(f"slice text lookup failed: {error}", file=sys.stderr)
        sys.exit(2)
