#!/usr/bin/env python3
"""Convert an upstream CommonMark / GFM ``spec.txt`` into the JSON fixture format.

The parser is a 1:1 port of the JavaScript extractor shipped with the
``commonmark-spec`` npm package (``index.js``), which is what MarkText's
original ``packages/muya`` test-suite consumes.  Keeping the semantics
identical means the example numbers line up with the upstream spec, so a
failure can be looked up in the CommonMark documentation directly.

GFM additions
-------------

The GFM specification marks the examples that need an extension with an
extension list on the example fence, for instance::

    ```````````````````````````````` example autolink

``cmark-gfm``'s own runner (``test/spec_tests.py``) reads that list and enables
exactly those extensions for exactly that example, which is why the plain
*Autolinks* section of the GFM spec still expects ``http://example.com`` to
stay plain text while the *Autolinks (extension)* section expects it to
become a link.  The extractor therefore records the list in the ``extensions``
field of every example so the Rust harness can do the same.

An example marked ``disabled`` is skipped by ``cmark-gfm`` altogether — the
reference implementation does not produce the HTML the specification shows for
it.  The marker is recorded as well so that the harness can skip it.

Usage::

    python scripts/convert-commonmark-spec.py <spec.txt> <out.json>

The CommonMark ``spec.txt`` is published with the npm tarball::

    npm pack commonmark-spec@0.31.2

The GFM ``spec.txt`` ships with the reference implementation::

    curl -LO https://raw.githubusercontent.com/github/cmark-gfm/master/test/spec.txt
"""

from __future__ import annotations

import json
import re
import sys

# Mirrors /extractSpecTests/ in commonmark-spec's index.js.
#
# The example fence may carry a list of extension names after the word
# "example" (GFM only); CommonMark never uses them.
EXAMPLE_RE = re.compile(
    r"^`{32} example(?P<extensions>[^\n]*)\n"  # an example block
    r"([\s\S]*?)^\.\n"  # the Markdown input
    r"([\s\S]*?)^`{32}$"  # the expected HTML
    r"|^#{1,6} *(.*)$",  # a section heading
    re.M,
)


def extract_spec_tests(data: str) -> list[dict]:
    """Pull every example out of a CommonMark / GFM ``spec.txt`` document."""
    examples: list[dict] = []
    current_section = ""
    example_number = 0

    # Normalise newlines for platform independence.
    text = re.sub(r"\r\n?", "\n", data)
    # Drop the trailing "END TESTS" region.
    text = re.sub(r"<!-- END TESTS -->.*", "", text, flags=re.S)

    for match in EXAMPLE_RE.finditer(text):
        markdown, html, section = match.group(2), match.group(3), match.group(4)
        if section is not None:
            current_section = section
            continue
        example_number += 1
        example = {
            "markdown": markdown,
            "html": html,
            "section": current_section,
            "number": example_number,
        }
        extensions = match.group(1).split()
        if extensions:
            example["extensions"] = extensions
        examples.append(example)
    return examples


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    source, target = argv[1], argv[2]
    with open(source, "r", encoding="utf-8") as handle:
        data = handle.read()
    examples = extract_spec_tests(data)
    with open(target, "w", encoding="utf-8") as handle:
        json.dump(examples, handle, ensure_ascii=False, indent=2)
        handle.write("\n")
    print(f"{len(examples)} examples -> {target}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
