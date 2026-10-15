// MP-08 / MP-10 / MP-11: parity with kernel skill/selection.rs literal exclusions.
import assert from "node:assert/strict";
import { test } from "node:test";
import { hasSkillReferences } from "./skills-client.js";
const literals = [
    "Explain the string \"hello $review world\"",
    "Explain 'hello $review world'",
    "    $review",
    "Paragraph\n    $review",
    "[read $review](https://example.com)",
    "[read](https://example.com/ $review )",
    "~~~\n$review\n~~~ not a closing fence\n$review",
    "``a ` $review``",
    "$HOME",
    "\\$review",
    "$20",
    "`$review`",
    "[label $review](url $review )",
    "    $review\n    $other",
    "$rrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrr",
    "$r\u00e9view",
    "${review}",
    "x$review",
    "($HOME)",
    "$review/path",
    "$review{",
    "```\n$review\n``` trailing\n$other"
];
const invocations = [
    "$review",
    "Read $review now",
    "($review)",
    "[$review]",
    "don't $review",
    "\"literal $review\" then $other",
    "    $hidden\n$visible",
    "[ignore $review](url) $other",
    "```\n$hidden\n```\n$visible",
    "~~~\n$hidden\n~~~~\n$visible",
    "$review.",
    "$Review",
    "$a_b-c",
    "$rrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrr"
];
for (const text of literals) test(`literal text uses ordinary submission: ${JSON.stringify(text)}`, () => assert.equal(hasSkillReferences(text), false));
for (const text of invocations) test(`explicit skill uses the resolver: ${JSON.stringify(text)}`, () => assert.equal(hasSkillReferences(text), true));
